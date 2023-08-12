use std::sync::{mpsc, Arc, Mutex};
use std::{fs::File, path::Path};
use std::{io, thread};

use audio_thread_priority::promote_current_thread_to_real_time;
use clap::Parser;
use symphonia::core::errors::Error;
use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, formats::FormatOptions, io::MediaSourceStream,
    meta::MetadataOptions, probe::Hint,
};
use tinyaudio::prelude::*;
use tracing_unwrap::ResultExt;
use wigglyair::configuration;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[clap(help = "File to play. Must be flac")]
    audio_file: String,
}

const SAMPLE_RATE: usize = 44100;
const CHANNEL_COUNT: usize = 2;

#[tokio::main]
async fn main() {
    // when this leaves scope the logs will be flushed
    let _guard = configuration::setup_tracing_async("play".into());

    let cli = Cli::parse();
    let audio_file = cli.audio_file;

    // from: https://github.com/pdeljanov/Symphonia/blob/master/symphonia/examples/basic-interleaved.rs
    let file = Box::new(File::open(Path::new(&audio_file)).unwrap_or_log());
    let mss = MediaSourceStream::new(file, Default::default());
    let hint = Hint::new();

    // Use the default options when reading and decoding.
    let format_opts: FormatOptions = Default::default();
    let metadata_opts: MetadataOptions = Default::default();
    let decoder_opts: DecoderOptions = Default::default();

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &format_opts, &metadata_opts)
        .unwrap();

    let mut format = probed.format;
    let track = format.default_track().unwrap();
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &decoder_opts)
        .unwrap();

    // Store the track identifier, we'll use it to filter packets.
    let track_id = track.id;

    let mut sample_buf = None;

    // create channel for individual samples (interleaved format)
    let (tx, rx) = mpsc::sync_channel::<f32>(SAMPLE_RATE / 5);

    // create channel for the sample rate of the first track
    let (rate_tx, rate_rx) = mpsc::sync_channel::<usize>(1);

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

                        rate_tx.send(spec.rate as usize).unwrap_or_log();

                        let duration = audio_buf.capacity() as u64;
                        tracing::info!(?duration, "Decoded audio buffer duration");

                        sample_buf = Some(SampleBuffer::<f32>::new(duration, spec));
                    }

                    // Copy the decoded audio buffer into the sample buffer in an interleaved format.
                    if let Some(buf) = &mut sample_buf {
                        buf.copy_interleaved_ref(audio_buf);
                        for sample in buf.samples() {
                            tx.send(*sample).unwrap();
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
    tracing::info!(
        audio_file,
        sample_rate,
        "Playing {audio_file} at {sample_rate} Hz"
    );

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
                *sample = rx.recv().unwrap_or_log();
            }
        }
    })
    .unwrap();

    loop {}
}
