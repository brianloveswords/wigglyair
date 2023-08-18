use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;
use std::{fs::File, path::Path};
use std::{io, thread};

use audio_thread_priority::promote_current_thread_to_real_time;
use clap::Parser;
use crossbeam::channel::{bounded, Sender};
use symphonia::core::errors::Error;
use symphonia::core::{audio::SampleBuffer, io::MediaSourceStream, probe::Hint};
use tinyaudio::prelude::*;
use tracing_unwrap::*;
use wigglyair::configuration;
use wigglyair::types::Volume;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[clap(help = "Files to play. Must be flac")]
    files: Vec<String>,

    // volumne
    #[clap(short, long, default_value = "100")]
    volume: u8,
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
struct AudioParams {
    duration: usize,
    channel_count: usize,
    sample_rate: usize,
}

impl AudioParams {
    const DEFAULT_AUDIO_BUFFER_FRAMES: u32 = 0;

    fn audio_buffer_frames(&self) -> u32 {
        Self::DEFAULT_AUDIO_BUFFER_FRAMES
    }

    fn channel_sample_count(&self) -> usize {
        self.sample_rate / 10
    }
}

impl From<AudioParams> for OutputDeviceParameters {
    fn from(other: AudioParams) -> Self {
        Self {
            channels_count: other.channel_count,
            sample_rate: other.sample_rate,
            channel_sample_count: other.channel_sample_count(),
        }
    }
}

struct PlayState(AtomicBool);

impl PlayState {
    fn new() -> Self {
        Self(AtomicBool::new(true))
    }

    fn toggle(&self) -> bool {
        self.0
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| Some(!v))
            .unwrap_or_log()
    }

    fn is_paused(&self) -> bool {
        !self.0.load(Ordering::SeqCst)
    }
}

impl Default for PlayState {
    fn default() -> Self {
        Self::new()
    }
}

fn main() {
    // when this leaves scope the logs will be flushed
    let _guard = configuration::setup_tracing_async("play".into());

    let cli = Cli::parse();
    let files = cli.files;
    let volume = {
        let v = Volume::try_from(cli.volume).expect_or_log("Could not parse volume");
        Arc::new(v)
    };
    let play_state = Arc::new(PlayState::default());

    let (samples_tx, samples_rx) = bounded::<Vec<f32>>(128);
    let (params_tx, params_rx) = bounded::<AudioParams>(1);

    let _ = start_volume_reader(volume.clone(), play_state.clone());
    let file_reader = start_file_reader(files, params_tx.clone(), samples_tx.clone());

    let params = params_rx.recv().unwrap_or_log();
    tracing::info!(?params, "Setting up audio device");
    let _device = run_output_device(params.into(), {
        // buffer to store samples that are ready to be played. we'll resize it to
        // the have enough capacity to hold what we need without reallocating.
        let mut buf: Vec<f32> = Vec::new();
        let volume = volume.clone();

        let mut initialized = false;
        move |data| {
            if !initialized {
                let tid = promote_current_thread_to_real_time(
                    params.audio_buffer_frames(),
                    params.sample_rate as u32,
                )
                .unwrap_or_log();
                tracing::info!(?tid, "Thread promoted");

                buf = Vec::with_capacity(data.len() * 2);
                initialized = true;
            }

            let size = data.len();

            // if we're paused, fill the buffer with zeros and get outta here
            // I tested just in case: using `fill` is about twice as fast as
            // using a static pile of zeroes and `copy_from_slice`.
            if play_state.is_paused() {
                data.fill(0.0);
                return;
            }

            let volume = volume.get();

            while buf.len() < size {
                let mut tmp = samples_rx
                    .recv()
                    .unwrap_or_log()
                    .iter()
                    .map(|s| s * (volume as f32 / 100.0))
                    .collect();
                buf.append(&mut tmp);
            }

            data.copy_from_slice(&buf[..size]);
            buf.drain(..size);
        }
    })
    .unwrap_or_log();

    file_reader.join().unwrap_or_log();
}

fn start_volume_reader(volume: Arc<Volume>, play_state: Arc<PlayState>) -> JoinHandle<()> {
    thread::spawn(move || loop {
        eprint!("> ");

        let mut msg = String::new();
        if let Err(error) = std::io::stdin().read_line(&mut msg) {
            tracing::error!("Error reading line: {}", error);
            break;
        }
        let command_line = msg.trim().split_whitespace().collect::<Vec<_>>();
        let command = match command_line.get(0) {
            Some(command) => *command,
            None => continue,
        };

        let tail = command_line[1..].to_vec();
        match command {
            "" => continue,
            "v" => {
                let value = tail.get(0).unwrap_or(&"");
                match volume.set_from_string(value) {
                    Ok(()) => tracing::info!(volume = value, "Setting volume"),
                    Err(error) => tracing::error!(?error, "Error setting volume"),
                }
            }
            "p" => {
                let playing = !play_state.toggle();
                tracing::info!(playing, "{}", if playing { "Playing" } else { "Pausing" });
            }
            _ => {}
        };

        if msg == "" {
            eprintln!();
            continue;
        }
    })
}

fn start_file_reader(
    files: Vec<String>,
    params_tx: Sender<AudioParams>,
    samples_tx: Sender<Vec<f32>>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut params_sent = false;
        for file in files {
            let path = Path::new(&file);
            if path.extension().unwrap_or_default() != "flac" {
                tracing::warn!(?path, "Skipping non-flac file");
                continue;
            }

            tracing::info!(audio_file = file, "Reading audio file");

            let probed = {
                let file = Box::new(File::open(path).unwrap_or_log());
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
                        tracing::error!(?err, file, "Error reading packet");
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
                            let channel_count = spec.channels.count();
                            let sample_rate = spec.rate as usize;

                            tracing::info!(?spec, "Decoded audio buffer");

                            if !params_sent {
                                params_tx
                                    .send(AudioParams {
                                        duration,
                                        channel_count,
                                        sample_rate,
                                    })
                                    .unwrap_or_log();
                                params_sent = true;
                            }

                            sample_buf = Some(SampleBuffer::new(duration as u64, spec));
                        }

                        if let Some(buf) = &mut sample_buf {
                            buf.copy_interleaved_ref(audio_buf);
                            loop {
                                match samples_tx.try_send(buf.samples().to_owned()) {
                                    // if the buffer is full, wait for a bit. this lets us
                                    // batch reads, which seems to be more efficient.
                                    Err(err) if err.is_full() => {
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
        }
    })
}
