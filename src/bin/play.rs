use std::time::Duration;
use std::{fs::File, path::Path};
use std::{io, thread};

use audio_thread_priority::promote_current_thread_to_real_time;
use clap::Parser;
use crossbeam::channel::{bounded, unbounded};
use symphonia::core::audio::SignalSpec;
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

    #[clap(short, long, default_value = "100", help = "Volume to play at")]
    volume: u8,

    #[clap(short, long, default_value = "0", help = "Number of samples to skip")]
    skip: u64,
}

const CHANNEL_COUNT: usize = 2;
const DEFAULT_AUDIO_BUFFER_FRAMES: u32 = 0;

struct SampleBufferSpec {
    buf: SampleBuffer<f32>,
}

impl SampleBufferSpec {
    fn new(duration: u64, spec: SignalSpec) -> Self {
        let buf = SampleBuffer::<f32>::new(duration, spec);
        Self::new_from_buffer(buf)
    }

    fn new_from_buffer(buf: SampleBuffer<f32>) -> Self {
        Self { buf }
    }
}

enum Event {
    AddTrackSamplePoint { file: String, samples: u64 },
}

#[derive(Debug)]
struct TrackSamplePoint {
    file: String,
    samples: u64,
}

fn main() {
    // when this leaves scope the logs will be flushed
    let _guard = configuration::setup_tracing_async("play".into());

    let cli = Cli::parse();

    let volume = cli.volume as f32 * 0.01;
    let files = cli.files;
    let skip = cli.skip;

    // channel for buffers of samples
    let (samples_tx, samples_rx) = bounded::<Vec<f32>>(256);

    // channel for the sample rate of the first track
    let (rate_tx, rate_rx) = bounded::<usize>(1);

    let (event_tx, event_rx) = unbounded::<Event>();

    let reader = thread::spawn(move || {
        let mut samples_read = 0u64;
        let mut skip_samples_remaining = skip;
        let mut rate_sent = false;
        for file in files {
            event_tx
                .send(Event::AddTrackSamplePoint {
                    file: file.clone(),
                    samples: samples_read,
                })
                .unwrap_or_log();

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

            let mut sample_buf: Option<SampleBufferSpec> = None;
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
                            tracing::debug!(?spec, "Decoded audio buffer spec");

                            // we need the sample rate to set up the audio device but
                            // we only need it once because we're not gonna change it
                            // ever again.
                            if !rate_sent {
                                rate_tx.send(spec.rate as usize).unwrap_or_log();
                                rate_sent = true;
                            }

                            let duration = audio_buf.capacity() as u64;
                            tracing::debug!(duration, "Decoded audio buffer duration");

                            sample_buf = Some(SampleBufferSpec::new(duration, spec));
                        }

                        if let Some(SampleBufferSpec { buf }) = &mut sample_buf {
                            buf.copy_interleaved_ref(audio_buf);
                            let mut buf_samples = buf.samples();
                            let buf_len = buf_samples.len() as u64;

                            if skip_samples_remaining > 0 {
                                let skip_samples = skip_samples_remaining.min(buf_len);
                                tracing::trace!(
                                    skip_samples_remaining,
                                    skip_samples,
                                    "Skipping samples"
                                );
                                skip_samples_remaining -= skip_samples;
                                if skip_samples == buf_len {
                                    samples_read += skip_samples;
                                    continue;
                                }
                                buf_samples = &buf_samples[0..skip_samples as usize];
                            }
                            let buf_samples_len = buf_samples.len();
                            let mut samples: Vec<f32> = Vec::with_capacity(buf_samples_len);
                            for sample in buf_samples {
                                samples.push(*sample * volume);
                            }

                            samples_read += buf_samples_len as u64;

                            loop {
                                match samples_tx.try_send(samples.clone()) {
                                    // if the buffer is full, wait for a bit. this lets us
                                    // batch reads, which seems to be more efficient.
                                    Err(err) if err.is_full() => {
                                        thread::sleep(Duration::from_secs(8));
                                    }
                                    Ok(_) => {
                                        tracing::trace!(samples = buf_samples_len, "Sent samples");
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
        let mut samples_processed = skip;
        let mut trackpoints: Vec<TrackSamplePoint> = vec![];

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

            if let Ok(event) = event_rx.try_recv() {
                match event {
                    Event::AddTrackSamplePoint { file, samples } => {
                        tracing::info!(?file, ?samples, "Adding track sample point");
                        trackpoints.push(TrackSamplePoint { file, samples });
                    }
                }
            }

            let size = data.len();

            while buf.len() < size {
                let mut buf2 = match samples_rx.recv() {
                    Ok(buf) => buf,
                    Err(err) => {
                        tracing::error!(?err, "Error receiving samples");
                        return;
                    }
                };
                buf.append(&mut buf2);
            }

            samples_processed += buf.len() as u64;
            data.copy_from_slice(&buf[..size]);
            buf.drain(..size);

            tracing::trace!(samples_processed, "Played {samples_processed} samples");
        }
    })
    .unwrap_or_log();

    reader.join().unwrap_or_log();
}
