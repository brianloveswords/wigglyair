use crate::configuration::Settings;
use audio_thread_priority::promote_current_thread_to_real_time;
use crossbeam::channel::{self, Sender, TryRecvError};
use itertools::FoldWhile::*;
use itertools::Itertools;
use metaflac::Tag;
use serde::Serialize;
use std::collections::HashSet;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::errors::Error;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::probe::Hint;
use tinyaudio::run_output_device;
use tinyaudio::OutputDeviceParameters;
use tracing_unwrap::*;

#[derive(Debug)]
pub struct AppState {
    pub settings: Settings,
}

pub type SharedState = Arc<AppState>;

#[derive(Debug, Serialize)]
pub struct DebugResponse {
    pub paths: Vec<String>,
}

//
// Volume
//

#[derive(Debug)]
pub enum VolumeError {
    InvalidValue(u8),
    InvalidString(String),
}

#[derive(Debug)]
pub struct Volume(AtomicU8);

impl Volume {
    const MAX: u8 = 100;

    fn unsafe_from(initial: u8) -> Self {
        Self(AtomicU8::new(initial))
    }

    /// Get the current volume
    pub fn get(&self) -> u8 {
        self.0.load(Ordering::Acquire)
    }

    /// Set the volume
    ///
    /// Returns an error if the value is greater than `Self::MAX`
    pub fn set(&self, value: u8) -> Result<(), VolumeError> {
        if value > Self::MAX {
            Err(VolumeError::InvalidValue(value))
        } else {
            self.0.store(value, Ordering::Release);
            Ok(())
        }
    }

    fn change(&self, value: i16) -> u8 {
        let mut ret = 0u8;
        self.0
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |prev| {
                let prev = prev as i16;
                let new = (prev + value as i16).clamp(0, 100);
                ret = new as u8;
                Some(new as u8)
            })
            .unwrap();
        ret
    }

    /// Increase the volume by `value`
    ///
    /// Returns the *previous* volume
    pub fn up(&self, value: u8) -> u8 {
        self.change(value as i16)
    }

    /// Decrease the volume by `value`
    ///
    /// Returns the *previous* volume
    pub fn down(&self, value: u8) -> u8 {
        self.change(-(value as i16))
    }

    pub fn set_from_string(&self, value: &str) -> Result<(), VolumeError> {
        let value: u8 = value
            .trim()
            .parse()
            .map_err(|_| VolumeError::InvalidString(value.to_owned()))?;
        self.set(value)
    }
}

impl Default for Volume {
    fn default() -> Self {
        Self::unsafe_from(Self::MAX)
    }
}

impl TryFrom<u8> for Volume {
    type Error = VolumeError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if value > Self::MAX {
            Err(VolumeError::InvalidValue(value))
        } else {
            Ok(Self::unsafe_from(value))
        }
    }
}

impl TryFrom<String> for Volume {
    type Error = VolumeError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let value: u8 = value
            .trim()
            .parse()
            .map_err(|_| VolumeError::InvalidString(value))?;
        Self::try_from(value)
    }
}

impl FromStr for Volume {
    type Err = VolumeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value.to_string())
    }
}

//
// Audio Params
//

#[derive(Debug, Clone, Copy)]
pub struct AudioParams {
    pub channel_count: usize,
    pub sample_rate: usize,
}

impl AudioParams {
    const DEFAULT_AUDIO_BUFFER_FRAMES: u32 = 0;

    pub fn audio_buffer_frames(&self) -> u32 {
        Self::DEFAULT_AUDIO_BUFFER_FRAMES
    }

    pub fn channel_sample_count(&self) -> usize {
        self.sample_rate / 10
    }

    pub fn output_device_parameters(&self) -> OutputDeviceParameters {
        OutputDeviceParameters {
            channels_count: self.channel_count,
            sample_rate: self.sample_rate,
            channel_sample_count: self.channel_sample_count(),
        }
    }
}

//
// TrackList
//

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub path: PathBuf,
    pub sample_rate: u32,
    pub samples: u64,
    pub channels: u8,
}

impl Track {
    fn from_path(path: &Path) -> Self {
        let tag = Tag::read_from_path(path).unwrap();
        let si = tag.get_streaminfo().unwrap();
        let samples = si.total_samples;
        let channels = si.num_channels;
        let sample_rate = si.sample_rate;
        Self {
            path: path.to_owned(),
            sample_rate,
            samples,
            channels,
        }
    }
}

#[derive(Debug)]
pub struct TrackList {
    pub tracks: Vec<Track>,
    pub total_samples: u64,
}

impl TrackList {
    pub fn new() -> Self {
        Self {
            tracks: Vec::new(),
            total_samples: 0,
        }
    }

    pub fn from_files(files: Vec<String>) -> Self {
        files
            .iter()
            .map(Path::new)
            .filter(is_flac)
            .map(|p| Track::from_path(p.canonicalize().unwrap().as_path()))
            .collect_vec()
            .into()
    }

    pub fn add_track(&mut self, track: Track) {
        self.total_samples += track.samples;
        self.tracks.push(track);
    }

    pub fn add_tracks(&mut self, tracks: Vec<Track>) {
        self.total_samples += tracks.iter().map(|t| t.samples).sum::<u64>();
        self.tracks.extend(tracks);
    }

    pub fn find_playing(&self, current_sample: u64) -> usize {
        let (found, _) = self
            .tracks
            .iter()
            .enumerate()
            .fold_while((0usize, 0u64), |(_, mut total), (i, track)| {
                total += track.samples;
                if total > current_sample {
                    Done((i, total))
                } else {
                    Continue((i, total))
                }
            })
            .into_inner();
        found
    }

    pub fn audio_params(&self) -> AudioParams {
        let channels = self
            .tracks
            .iter()
            .map(|t| t.channels)
            .collect::<HashSet<_>>();

        let sample_rates = self
            .tracks
            .iter()
            .map(|t| t.sample_rate)
            .collect::<HashSet<_>>();

        // TODO: don't panic, warn the user of the problem and give them
        // a suggestion on how to fix it. include an `--allow-resampling`
        // flag and figure out how to resample the audio?

        assert!(
            channels.len() == 1,
            "Multiple channel counts found in track list: {:?}",
            channels
        );

        assert!(
            sample_rates.len() == 1,
            "Multiple samples rates found in track list: {:?}",
            sample_rates
        );

        AudioParams {
            channel_count: *channels.iter().next().unwrap() as usize,
            sample_rate: *sample_rates.iter().next().unwrap() as usize,
        }
    }
}

fn is_flac(p: &&Path) -> bool {
    p.exists() && p.extension().unwrap_or_default() == "flac"
}

impl Default for TrackList {
    fn default() -> Self {
        Self::new()
    }
}

impl Into<TrackList> for Vec<Track> {
    fn into(self) -> TrackList {
        let mut tl = TrackList::new();
        tl.add_tracks(self);
        tl
    }
}

//
// Player
//

pub struct Player {
    pub current_sample: Arc<AtomicU64>,
    pub total_samples: Arc<AtomicU64>,
    pub state: Arc<PlayState>,
    pub volume: Arc<Volume>,
    pub track_list: Arc<TrackList>,
    pub current_track: Arc<AtomicUsize>,
    pub audio_params: Arc<AudioParams>,
}

impl Player {
    pub fn new(track_list: TrackList) -> Self {
        Self {
            current_sample: Arc::new(AtomicU64::new(0)),
            volume: Arc::new(Volume::default()),
            state: Arc::new(PlayState::new()),
            total_samples: Arc::new(AtomicU64::new(track_list.total_samples)),
            current_track: Arc::new(AtomicUsize::new(0)),
            audio_params: Arc::new(track_list.audio_params()),
            track_list: Arc::new(track_list),
        }
    }

    pub fn start(self) -> JoinHandle<()> {
        let track_list = self.track_list.clone();
        let params = self.audio_params.clone();
        let current_sample = self.current_sample.clone();
        let channel_count = params.channel_count;
        let current_track = self.current_track.clone();
        let play_state = self.state.clone();
        let volume = self.volume.clone();
        let (samples_tx, samples_rx) = channel::bounded::<Vec<f32>>(100);
        let paths = track_list
            .tracks
            .iter()
            .map(|t| t.path.clone())
            .collect_vec();

        let (done_tx, done_rx) = channel::bounded::<()>(0);
        thread::spawn(move || {
            let reader_handle = start_file_reader(paths, samples_tx);

            // buffer to store samples that are ready to be played. we'll resize it to
            // the have enough capacity to hold what we need without reallocating.
            let mut buf: Vec<f32> = Vec::new();

            let mut initialized = false;
            let mut is_done = false;
            tracing::info!(?params, "Setting up audio device");
            let _device = run_output_device(params.output_device_parameters(), move |data| {
                if play_state.is_paused() || is_done {
                    data.fill(0.0);
                    return;
                }

                let size = data.len();

                if !initialized {
                    let tid = promote_current_thread_to_real_time(
                        params.audio_buffer_frames(),
                        params.sample_rate as u32,
                    )
                    .unwrap_or_log();
                    tracing::info!(?tid, "Thread promoted");

                    buf = Vec::with_capacity(size * 2);
                    initialized = true;
                }

                let volume = volume.get();

                while buf.len() < size {
                    match samples_rx.try_recv() {
                        Ok(samples) => {
                            tracing::trace!(
                                buf_len = buf.len(),
                                size,
                                samples_len = samples.len(),
                                "Buffering samples"
                            );
                            let mut tmp = samples
                                .iter()
                                .map(|s| s * (volume as f32 / 100.0))
                                .collect();
                            buf.append(&mut tmp);
                        }
                        Err(TryRecvError::Empty) => {
                            tracing::warn!("Samples channel empty");
                            break;
                        }
                        Err(TryRecvError::Disconnected) => {
                            tracing::info!("Samples channel disconnected");
                            if let Err(error) = done_tx.send(()) {
                                tracing::error!(?error, "Error sending done signal");
                            }
                            is_done = true;
                            break;
                        }
                    }
                }

                // the last buffer is unlikely to be perfectly full. if we're on the
                // last buffer we go through the extra work of making sure the slice
                // is zero-padded to the right size. this involves extra allocations
                // so it's worth the tax of checking this boolean every callback.
                let max = size.min(buf.len());
                let slice = &buf[..max];
                if max == size {
                    data.copy_from_slice(&slice);
                } else {
                    if !is_done {
                        tracing::warn!(
                            max,
                            size,
                            buf_len = buf.len(),
                            "Buffer not full; padding with zeroes",
                        );
                    }
                    let mut tmp = Vec::with_capacity(size);
                    tmp.extend_from_slice(slice);
                    tmp.resize(size, 0.0);
                    data.copy_from_slice(&tmp);
                }

                buf.drain(..max);

                let sample_count =
                    current_sample.fetch_add(max as u64 / channel_count as u64, Ordering::SeqCst);

                let track = track_list.find_playing(sample_count);
                current_track.store(track, Ordering::SeqCst);
            })
            .unwrap_or_log();

            reader_handle.join().unwrap_or_log();
            done_rx.recv().unwrap_or_log();
            tracing::info!("Player finished");
        })
    }
}

fn start_file_reader(paths: Vec<PathBuf>, samples_tx: Sender<Vec<f32>>) -> JoinHandle<()> {
    let mut total_samples = 0;
    thread::spawn(move || {
        for path in paths {
            if path.extension().unwrap_or_default() != "flac" {
                tracing::warn!(?path, "Skipping non-flac file");
                continue;
            }

            tracing::info!(?path, "Reading audio file");

            let probed = {
                let file = Box::new(File::open(&path).unwrap_or_log());
                symphonia::default::get_probe()
                    .format(
                        &Hint::new(),
                        MediaSourceStream::new(file, Default::default()),
                        &Default::default(),
                        &Default::default(),
                    )
                    .unwrap_or_log()
            };

            let mut format = probed.format;
            let track = format.default_track().unwrap_or_log();

            let mut decoder = symphonia::default::get_codecs()
                .make(&track.codec_params, &Default::default())
                .unwrap_or_log();

            let track_id = track.id;

            let mut sample_buf = None;
            loop {
                let packet = match format.next_packet() {
                    Ok(packet) => packet,
                    Err(Error::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                    Err(err) => {
                        tracing::error!(?err, ?path, "Error reading packet");
                        break;
                    }
                };

                if packet.track_id() != track_id {
                    continue;
                }

                match decoder.decode(&packet) {
                    Ok(audio_buf) => {
                        if sample_buf.is_none() {
                            let spec = *audio_buf.spec();
                            let duration = audio_buf.capacity();
                            tracing::info!(?spec, "Decoded audio buffer");
                            sample_buf = Some(SampleBuffer::new(duration as u64, spec));
                        }

                        if let Some(buf) = &mut sample_buf {
                            buf.copy_interleaved_ref(audio_buf);
                            let mut samples = buf.samples().to_owned();
                            total_samples += samples.len() as u64;
                            loop {
                                match samples_tx.try_send(samples) {
                                    // if the buffer is full, wait for a bit. this lets us
                                    // batch reads, which seems to be more efficient.
                                    Err(err) if err.is_full() => {
                                        samples = err.into_inner();
                                        thread::sleep(Duration::from_secs(4));
                                    }
                                    Ok(_) => {
                                        tracing::trace!("Sent samples");
                                        break;
                                    }
                                    Err(err) => {
                                        tracing::error!(?err, "Error sending samples");
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    Err(Error::DecodeError(err)) => {
                        tracing::error!(err, "Audio loop: decode error")
                    }
                    Err(err) => {
                        tracing::error!(%err, "Audio loop: error");
                        break;
                    }
                }
            }
            tracing::info!(total_samples, ?path, "Finished reading file");
        }
    })
}

//
// PlayState
//

pub struct PlayState(AtomicBool);

impl PlayState {
    pub fn new() -> Self {
        Self(AtomicBool::new(true))
    }

    /// Toggle the play state.
    ///
    /// Returns the *previous* state.
    pub fn toggle(&self) -> bool {
        self.0
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| Some(!v))
            .unwrap_or_log()
    }

    pub fn is_paused(&self) -> bool {
        !self.0.load(Ordering::SeqCst)
    }
}

impl Default for PlayState {
    fn default() -> Self {
        Self::new()
    }
}

//
// tests
//

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn test_volume_up_stays_below_100(amount: u8) {
            let v = Volume::default();
            let result = v.up(amount);
            prop_assert!(result <= 100);
        }

        #[test]
        fn test_volume_down_stays_below_100(amount: u8) {
            let v = Volume::default();
            let result = v.down(amount);
            prop_assert!(result <= 100);
        }
    }
}
