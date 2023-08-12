use std::{fs::File, path::Path};
use std::{io, thread};

use audio_thread_priority::promote_current_thread_to_real_time;
use clap::Parser;
use crossbeam::channel::bounded;
use symphonia::core::errors::Error;
use symphonia::core::{audio::SampleBuffer, io::MediaSourceStream, probe::Hint};
use tinyaudio::prelude::*;
use tracing_unwrap::ResultExt;
use wigglyair::configuration;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[clap(help = "First file to play. Must be flac")]
    file1: String,

    #[clap(help = "Second file to play. Must be flac")]
    file2: String,
}

const SAMPLE_RATE: usize = 44100;
const CHANNEL_COUNT: usize = 2;

fn main() {
    // when this leaves scope the logs will be flushed
    let _guard = configuration::setup_tracing_async("play".into());

    let cli = Cli::parse();
    let file1 = cli.file1;
    let _file2 = cli.file2;

    // create channel for individual samples (interleaved format)
    let (sample_tx, sample_rx) = bounded::<f32>(SAMPLE_RATE * CHANNEL_COUNT * 30);

    // create channel for the sample rate of the first track
    let (rate_tx, rate_rx) = bounded::<usize>(1);
    let mut rate_tx = Some(rate_tx);

    let probed = {
        let file = Box::new(File::open(Path::new(&file1)).unwrap_or_log());
        symphonia::default::get_probe()
            .format(
                &Hint::new(),
                MediaSourceStream::new(file, Default::default()),
                &Default::default(),
                &Default::default(),
            )
            .unwrap()
    };

    let mut format = probed.format;
    let track = format.default_track().unwrap();

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &Default::default())
        .unwrap();

    // Store the track identifier, we'll use it to filter packets.
    let track_id = track.id;

    let mut sample_buf = None;

    thread::spawn(move || {
        loop {
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(Error::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(err) => {
                    tracing::error!(?err, "Error reading packet");
                    panic!("Error reading packet")
                }
            };

            // If the packet does not belong to the selected track, skip it.
            if packet.track_id() != track_id {
                continue;
            }

            match decoder.decode(&packet) {
                Ok(audio_buf) => {
                    if sample_buf.is_none() {
                        let spec = *audio_buf.spec();
                        tracing::info!(?spec, "Decoded audio buffer spec");

                        if let Some(rate_tx) = rate_tx.take() {
                            rate_tx.send(spec.rate as usize).unwrap_or_log();
                        }

                        let duration = audio_buf.capacity() as u64;
                        tracing::info!(?duration, "Decoded audio buffer duration");

                        sample_buf = Some(SampleBuffer::<f32>::new(duration, spec));
                    }

                    // Copy the decoded audio buffer into the sample buffer in an interleaved format.
                    if let Some(buf) = &mut sample_buf {
                        buf.copy_interleaved_ref(audio_buf);
                        for sample in buf.samples() {
                            sample_tx.send(*sample).unwrap();
                        }
                    }
                }
                Err(Error::DecodeError(err)) => tracing::error!(?err, "audio loop: decode error"),
                Err(err) => {
                    tracing::error!(?err, "audio loop: error");
                    break;
                }
            }
        }
    });

    let sample_rate = rate_rx.recv().unwrap_or_log();
    tracing::info!(file1, sample_rate, "Playing {file1} at {sample_rate} Hz");

    let params = OutputDeviceParameters {
        channels_count: CHANNEL_COUNT,
        sample_rate,
        channel_sample_count: sample_rate / 10,
    };

    let _device = run_output_device(params, {
        let mut promoted = false;
        move |data| {
            if !promoted {
                let tid =
                    promote_current_thread_to_real_time(0, SAMPLE_RATE as u32).unwrap_or_log();
                tracing::info!(?tid, "thread promoted");
                promoted = true;
            }
            for sample in data.iter_mut() {
                *sample = sample_rx.recv().unwrap_or_log();
            }
        }
    })
    .unwrap();

    loop {}
}
