use std::io;
use std::{fs::File, path::Path};

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
    let mut full_audio_buf = Vec::<f32>::new();
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
                // The decoded audio samples may now be accessed via the audio buffer if per-channel
                // slices of samples in their native decoded format is desired. Use-cases where
                // the samples need to be accessed in an interleaved order or converted into
                // another sample format, or a byte buffer is required, are covered by copying the
                // audio buffer into a sample buffer or raw sample buffer, respectively. In the
                // example below, we will copy the audio buffer into a sample buffer in an
                // interleaved order while also converting to a f32 sample format.

                // If this is the *first* decoded packet, create a sample buffer matching the
                // decoded audio buffer format.
                if sample_buf.is_none() {
                    // Get the audio buffer specification.
                    let spec = *audio_buf.spec();
                    tracing::info!(?spec, "Decoded audio buffer spec");

                    // Get the capacity of the decoded buffer. Note: This is capacity, not length!
                    let duration = audio_buf.capacity() as u64;
                    tracing::info!(?duration, "Decoded audio buffer duration");

                    // Create the f32 sample buffer.
                    sample_buf = Some(SampleBuffer::<f32>::new(duration, spec));
                }

                // Copy the decoded audio buffer into the sample buffer in an interleaved format.
                if let Some(buf) = &mut sample_buf {
                    buf.copy_interleaved_ref(audio_buf);
                    full_audio_buf.extend_from_slice(buf.samples());
                }
            }
            Err(Error::DecodeError(err)) => tracing::error!(?err, "audio loop: decode error"),
            Err(err) => {
                tracing::error!(?err, "audio loop: error");
                break;
            }
        }
    }

    let full_audio_buf_len = full_audio_buf.len();
    tracing::info!(?full_audio_buf_len, "sample buffer length");
    tracing::info!(audio_file, "Playing {audio_file}");

    let params = OutputDeviceParameters {
        channels_count: 2,
        sample_rate: 44100,
        channel_sample_count: 4410,
    };

    let _device = run_output_device(params, {
        let mut promoted = false;
        move |data| {
            if !promoted {
                let tid = promote_current_thread_to_real_time(0, 44100).unwrap_or_log();
                tracing::info!(?tid, "thread promoted");
                promoted = true;
            }

            let size = data.len();
            let remaining = full_audio_buf.len();
            tracing::info!(remaining, "Remaining samples: {}", remaining);

            let drain_size = std::cmp::min(size, remaining);
            let mut sample_buf = full_audio_buf.drain(..drain_size).collect::<Vec<_>>();

            // zero pad the rest and copy to output buffer
            sample_buf.resize(size, 0.0);
            data.copy_from_slice(&sample_buf);
        }
    })
    .unwrap();

    loop {}
}
