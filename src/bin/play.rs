use std::time::Duration;
use std::{fs::File, path::Path};
use std::{io, thread};

use audio_thread_priority::promote_current_thread_to_real_time;
use clap::Parser;
use crossbeam::channel::bounded;
use symphonia::core::errors::Error;
use symphonia::core::{audio::SampleBuffer, io::MediaSourceStream, probe::Hint};
use tinyaudio::prelude::*;
use tracing_unwrap::*;
use wigglyair::configuration;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[clap(help = "Files to play. Must be flac")]
    files: Vec<String>,
}

const CHANNEL_COUNT: usize = 2;
const DEFAULT_AUDIO_BUFFER_FRAMES: u32 = 0;

fn main() {
    // when this leaves scope the logs will be flushed
    let _guard = configuration::setup_tracing_async("play".into());

    let cli = Cli::parse();
    let files = cli.files;

    // channel for buffers of samples
    let (samples_tx, samples_rx) = bounded::<Vec<f32>>(256);

    // channel for the sample rate of the first track
    let (rate_tx, rate_rx) = bounded::<usize>(1);

    let reader = thread::spawn(move || {
        let mut rate_sent = false;
        for file in files {
            let path = Path::new(&file);
            if path.extension().unwrap_or_default() != "flac" {
                tracing::error!(?path, "Skipping non-flac file");
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
                            tracing::info!(?spec, "Decoded audio buffer spec");

                            // we need the sample rate to set up the audio device but
                            // we only need it once because we're not gonna change it
                            // ever again.
                            if !rate_sent {
                                rate_tx.send(spec.rate as usize).unwrap_or_log();
                                rate_sent = true;
                            }

                            let duration = audio_buf.capacity() as u64;
                            tracing::info!(?duration, "Decoded audio buffer duration");

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
    });

    let sample_rate = rate_rx.recv().unwrap_or_log();
    tracing::info!(sample_rate, "Playing at {sample_rate} Hz");

    let params = OutputDeviceParameters {
        channels_count: CHANNEL_COUNT,
        sample_rate,
        channel_sample_count: sample_rate / 10,
    };

    let _device = run_output_device(params, {
        let mut promoted = false;
        let mut buf: Vec<f32> = Vec::new();
        move |data| {
            let size = data.len();
            if !promoted {
                let tid = promote_current_thread_to_real_time(
                    DEFAULT_AUDIO_BUFFER_FRAMES,
                    sample_rate as u32,
                )
                .unwrap_or_log();
                tracing::info!(?tid, "Thread promoted");
                promoted = true;
            }
            while buf.len() < size {
                let mut buf2 = samples_rx.recv().unwrap_or_log();
                buf.append(&mut buf2);
            }
            data.copy_from_slice(&buf[..size]);
            buf.drain(..size);
        }
    })
    .unwrap_or_log();

    reader.join().unwrap_or_log();
}
