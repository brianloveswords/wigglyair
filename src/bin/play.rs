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

#[derive(Debug)]
struct AudioDeviceParams {
    channel_count: usize,
    sample_rate: usize,
    channel_sample_count: usize,
}

impl From<AudioDeviceParams> for OutputDeviceParameters {
    fn from(other: AudioDeviceParams) -> Self {
        Self {
            channels_count: other.channel_count,
            sample_rate: other.sample_rate,
            channel_sample_count: other.channel_sample_count,
        }
    }
}

const DEFAULT_AUDIO_BUFFER_FRAMES: u32 = 0;

fn main() {
    // when this leaves scope the logs will be flushed
    let _guard = configuration::setup_tracing_async("play".into());

    let cli = Cli::parse();
    let files = cli.files;
    let volume = {
        let v = Volume::try_from(cli.volume).expect_or_log("Could not parse volume");
        Arc::new(v)
    };

    // channel for buffers of samples
    let (samples_tx, samples_rx) = bounded::<Vec<f32>>(256);

    // channel for the output parameters
    let (params_tx, params_rx) = bounded::<AudioDeviceParams>(1);

    let _ = start_volume_reader(volume.clone());
    let file_reader = start_file_reader(files, params_tx.clone(), samples_tx.clone());

    let params = params_rx.recv().unwrap_or_log();
    let sample_rate = params.sample_rate;
    tracing::info!(?params, "Setting up audio device");
    let _device = run_output_device(params.into(), {
        let mut promoted = false;
        let mut buf: Vec<f32> = Vec::new();
        let volume = volume.clone();
        move |data| {
            if !promoted {
                let tid = promote_current_thread_to_real_time(
                    DEFAULT_AUDIO_BUFFER_FRAMES,
                    sample_rate as u32,
                )
                .unwrap_or_log();
                tracing::info!(?tid, "Thread promoted");
                promoted = true;
            }

            let volume = volume.get();
            let size = data.len();

            while buf.len() < size {
                let mut buf2: Vec<f32> = samples_rx
                    .recv()
                    .unwrap_or_log()
                    .iter()
                    .map(|s| s * (volume as f32 / 100.0))
                    .collect();
                buf.append(&mut buf2);
            }
            tracing::trace!(volume, size, "Rendering samples");
            data.copy_from_slice(&buf[..size]);
            buf.drain(..size);
        }
    })
    .unwrap_or_log();

    file_reader.join().unwrap_or_log();
}

fn start_volume_reader(volume: Arc<Volume>) -> JoinHandle<()> {
    thread::spawn(move || loop {
        eprint!("> ");

        let mut msg = String::new();
        if let Err(error) = std::io::stdin().read_line(&mut msg) {
            tracing::error!("Error reading line: {}", error);
            break;
        }
        if msg == "" {
            eprintln!();
            continue;
        }
        match volume.set_from_string(&msg) {
            Ok(()) => tracing::info!(volume = msg, "Setting volume"),
            Err(error) => tracing::error!(?error, "Error setting volume"),
        }
    })
}

fn start_file_reader(
    files: Vec<String>,
    params_tx: Sender<AudioDeviceParams>,
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

            // store the track id so we can filter packets later.
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
                            let duration = audio_buf.capacity() as u64;

                            tracing::info!(?spec, duration, "Decoded audio buffer");

                            if !params_sent {
                                params_tx
                                    .send(AudioDeviceParams {
                                        channel_count: spec.channels.count(),
                                        sample_rate: spec.rate as usize,
                                        channel_sample_count: spec.rate as usize / 10,
                                    })
                                    .unwrap_or_log();
                                params_sent = true;
                            }

                            sample_buf = Some(SampleBuffer::new(duration, spec));
                        }

                        if let Some(buf) = &mut sample_buf {
                            buf.copy_interleaved_ref(audio_buf);
                            loop {
                                match samples_tx.try_send(buf.samples().to_owned()) {
                                    // if the buffer is full, wait for a bit. this lets us
                                    // batch reads, which seems to be more efficient.
                                    Err(err) if err.is_full() => {
                                        thread::sleep(Duration::from_secs(8));
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
