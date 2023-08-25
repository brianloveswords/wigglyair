use crate::configuration::Settings;
use crate::files;
use audio_thread_priority::promote_current_thread_to_real_time;
use crossbeam::channel::{self, Sender, TryRecvError};
use itertools::FoldWhile::{Continue, Done};
use itertools::Itertools;
use metaflac::Tag;
use serde::Serialize;
use std::collections::HashSet;
use std::fs::File;
use std::io;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::io::MediaSourceStreamOptions;
use symphonia::core::meta::MetadataOptions;
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
    /// # Errors
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
                let prev = i16::from(prev);
                let new = (prev + value).clamp(0, 100);
                ret = u8::try_from(new).unwrap_or_log();
                Some(u8::try_from(new).unwrap_or_log())
            })
            .unwrap();
        ret
    }

    /// Increase the volume by `value`
    ///
    /// Returns the *previous* volume
    pub fn up(&self, value: u8) -> u8 {
        self.change(i16::from(value))
    }

    /// Decrease the volume by `value`
    ///
    /// Returns the *previous* volume
    pub fn down(&self, value: u8) -> u8 {
        self.change(-i16::from(value))
    }

    /// Set the volume from a string
    ///
    /// # Errors
    ///
    /// Returns an error if the string cannot be parsed as a u8 or
    /// if the value is greater than `Self::MAX`
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
    pub channel_count: u8,
    pub sample_rate: u32,
}

impl AudioParams {
    const DEFAULT_AUDIO_BUFFER_FRAMES: u32 = 0;

    pub fn audio_buffer_frames(self) -> u32 {
        Self::DEFAULT_AUDIO_BUFFER_FRAMES
    }

    fn channel_sample_count(self) -> u32 {
        self.sample_rate / 10
    }

    pub fn output_device_parameters(self) -> OutputDeviceParameters {
        OutputDeviceParameters {
            channels_count: usize::from(self.channel_count),
            sample_rate: usize::try_from(self.sample_rate).expect_or_log("Invalid sample rate"),
            channel_sample_count: usize::try_from(self.channel_sample_count())
                .expect_or_log("Invalid channel sample count"),
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
    pub album: String,
    pub album_artist: String,
    pub title: String,
    pub track: u32,
}

impl Track {
    fn from_path(path: PathBuf) -> Self {
        let tag = Tag::read_from_path(&path).unwrap();
        let si = tag.get_streaminfo().unwrap();
        let samples = si.total_samples;
        let channels = si.num_channels;
        let sample_rate = si.sample_rate;

        let comments = tag.vorbis_comments().unwrap_or_else(|| {
            tracing::error!(?path, "File missing vorbis comments");
            panic!("Missing comments: {}", path.display())
        });

        let title = comments
            .title()
            .and_then(|v| v.first().cloned())
            .unwrap_or_else(|| {
                tracing::error!(?path, "File missing title metadata");
                panic!("Missing title: {}", path.display())
            });

        let album = comments
            .album()
            .and_then(|v| v.first().cloned())
            .unwrap_or_else(|| {
                tracing::error!(?path, "File missing album metadata");
                panic!("Missing album: {}", path.display())
            });

        let album_artist = comments
            .album_artist()
            .and_then(|v| v.first().cloned())
            .unwrap_or_else(|| {
                tracing::error!(?path, "File missing album_artist metadata");
                panic!("Missing album_artist: {}", path.display())
            });

        let track = comments.track().unwrap_or_else(|| {
            tracing::error!(?path, "File missing track");
            panic!("Missing track")
        });

        Self {
            path,
            sample_rate,
            samples,
            channels,
            album,
            album_artist,
            title,
            track,
        }
    }
}

#[derive(Debug)]
pub struct TrackList {
    pub tracks: Vec<Track>,
    pub total_samples: u64,
}

// TODO: this needs work. In order to call something like `audio_params()` the track
// list must not be empty. This is a problem because it's currently possible to construct
// an empty track list in a few ways. We should probably make a smart constructor for this
// that returns a Result and disallow empty track lists. In the meantime, I will rename
// the constructors to include `unsafe_`.
impl TrackList {
    /// Create a new empty track list
    ///
    /// # Safety
    ///
    /// This method is unsafe because methods on this struct will panic if the track list
    /// is empty, so the user of this function must ensure correct initialization before
    /// use, or else the program will panic.
    pub fn unsafe_new() -> Self {
        Self {
            tracks: Vec::new(),
            total_samples: 0,
        }
    }

    /// Create a new track list from a list of files
    ///
    /// # Safety
    ///
    /// This function is unsafe because it may lead to a partially constructed
    /// `TrackList`. If all files get filtered out because they are unsupported,
    /// calls to some associated functions will panic.
    pub fn unsafe_from_files(filenames: &[String]) -> Self {
        files::only_audio(filenames)
            .into_iter()
            .map(Track::from_path)
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

    pub fn get_start_point(&self, index: usize) -> u64 {
        self.tracks
            .iter()
            .take(index)
            .map(|t| t.samples)
            .sum::<u64>()
    }

    /// Get the track by it's **0-based index** in the track list
    ///
    /// # Panics
    ///
    /// This function will panic if the index is out of bounds
    pub fn get_track(&self, index: usize) -> &Track {
        assert!(index < self.tracks.len(), "Index out of bounds");
        &self.tracks[index]
    }

    pub fn get_end_point(&self, index: usize) -> u64 {
        self.get_start_point(index) + self.get_sample_count(index)
    }

    /// Get the number of samples in the track at the given index
    pub fn get_sample_count(&self, index: usize) -> u64 {
        self.get_track(index).samples
    }

    /// Get the audio params for this track list
    ///
    /// # Panics
    ///
    /// This function will panic if the track list is empty, or if there are
    /// tracks with different sample rates.
    #[must_use]
    pub fn audio_params(&self) -> AudioParams {
        let tracks = &self.tracks;
        let channels = tracks.iter().map(|t| t.channels).collect::<HashSet<_>>();
        let sample_rates = tracks.iter().map(|t| t.sample_rate).collect::<HashSet<_>>();

        // TODO: don't panic, warn the user of the problem and give them
        // a suggestion on how to fix it. include an `--allow-resampling`
        // flag and figure out how to resample the audio?

        assert!(
            channels.len() == 1,
            "Multiple channel counts found in track list: {tracks:?}"
        );

        assert!(
            sample_rates.len() == 1,
            "Multiple samples rates found in track list: {tracks:?}"
        );

        AudioParams {
            channel_count: *channels.iter().next().expect_or_log("No channels found"),
            sample_rate: *sample_rates
                .iter()
                .next()
                .expect_or_log("No sample rate found"),
        }
    }

    pub fn get_bounds(&self, i: usize) -> (u64, u64) {
        let start = self.get_start_point(i);
        let end = start + self.get_sample_count(i);
        (start, end)
    }
}

impl Default for TrackList {
    fn default() -> Self {
        Self::unsafe_new()
    }
}

impl From<Vec<Track>> for TrackList {
    fn from(v: Vec<Track>) -> Self {
        let mut tl = Self::unsafe_new();
        tl.add_tracks(v);
        tl
    }
}

//
// CurrentSample
//

pub struct CurrentSample(AtomicU64);

impl CurrentSample {
    fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    fn get_and_advance(&self, samples: u64) -> u64 {
        self.0.fetch_add(samples, Ordering::SeqCst)
    }

    pub fn get(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

impl Default for CurrentSample {
    fn default() -> Self {
        Self::new()
    }
}

//
// Player
//

pub struct Player {
    pub current_sample: Arc<CurrentSample>,
    pub total_samples: Arc<AtomicU64>,
    pub state: Arc<PlayState>,
    pub volume: Arc<Volume>,
    pub track_list: Arc<TrackList>,
    pub current_track: Arc<AtomicUsize>,
    pub audio_params: Arc<AudioParams>,
}

impl Player {
    pub fn new(track_list: TrackList) -> Self {
        Self::with_state(track_list, PlayState::with_state(true))
    }

    pub fn with_state(track_list: TrackList, state: PlayState) -> Self {
        Self {
            current_sample: Arc::new(CurrentSample::default()),
            volume: Arc::new(Volume::default()),
            state: Arc::new(state),
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
                        params.sample_rate,
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
                                .map(|s| s * (f32::from(volume) / 100.0))
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
                    data.copy_from_slice(slice);
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
                    current_sample.get_and_advance(max as u64 / u64::from(channel_count));

                let track = track_list.find_playing(sample_count);
                current_track.store(track, Ordering::SeqCst);
            })
            .unwrap_or_log();

            reader_handle
                .join()
                .expect_or_log("Error joining reader thread");
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
                        MediaSourceStream::new(file, MediaSourceStreamOptions::default()),
                        &FormatOptions::default(),
                        &MetadataOptions::default(),
                    )
                    .unwrap_or_log()
            };

            let mut format = probed.format;
            let track = format.default_track().unwrap_or_log();

            let mut decoder = symphonia::default::get_codecs()
                .make(&track.codec_params, &DecoderOptions::default())
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
                            sample_buf = Some((spec, SampleBuffer::new(duration as u64, spec)));
                        }

                        if let Some((_spec, buf)) = &mut sample_buf {
                            buf.copy_interleaved_ref(audio_buf);
                            let mut samples = buf.samples().to_owned();
                            let duration = samples.len();
                            total_samples += duration as u64;

                            // try to send the sample buffer. if the channel is full, wait for
                            // a bit. this lets us batch reads, which seems to be more efficient.
                            loop {
                                match samples_tx.try_send(samples) {
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
                        tracing::error!(err, "Audio loop: decode error");
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

/// Player state.
///
/// `true` represents playing
pub struct PlayState(AtomicBool);

impl PlayState {
    #[must_use]
    /// Create a new `PlayState` in the playing state.
    pub fn new() -> Self {
        Self::with_state(true)
    }

    #[must_use]
    /// Create a new `PlayState` with the given state.
    pub fn with_state(playing: bool) -> Self {
        Self(AtomicBool::new(playing))
    }

    /// Toggle the play state.
    ///
    /// Returns the *previous* state.
    pub fn toggle(&self) -> bool {
        self.0
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| Some(!v))
            .unwrap_or_log()
    }

    /// Whether the player is currently playing.
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
