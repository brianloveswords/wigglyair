use audio_thread_priority::{
    demote_current_thread_from_real_time, promote_current_thread_to_real_time,
};
use clap::Parser;
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

    tracing::info!(audio_file, "Playing {audio_file}");

    let params = OutputDeviceParameters {
        channels_count: 2,
        sample_rate: 44100,
        channel_sample_count: 4410,
    };

    // play a sine wave
    let _device = run_output_device(params, {
        let mut clock = 0f32;
        move |data| {
            tracing::info!("Writing {} samples", data.len());
            promote_current_thread_to_real_time(1024, 44100).unwrap_or_log();
            for samples in data.chunks_mut(params.channels_count) {
                clock = (clock + 1.0) % params.sample_rate as f32;
                let value =
                    (clock * 440.0 * 2.0 * std::f32::consts::PI / params.sample_rate as f32).sin();
                for sample in samples {
                    *sample = value;
                }
            }
        }
    })
    .unwrap();

    std::thread::sleep(std::time::Duration::from_secs(30));
}
