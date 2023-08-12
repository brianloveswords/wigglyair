use clap::Parser;
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
    let _guard = configuration::setup_tracing_async("play".into());

    let cli = Cli::parse();
    let audio_file = cli.audio_file;

    tracing::info!(audio_file, "Playing {audio_file}");
    Err("trying this out").unwrap_or_log()
}
