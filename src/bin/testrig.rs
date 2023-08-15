use clap::Parser;
use walkdir::WalkDir;
use wigglyair::configuration;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[clap(help = "Root to start walking from")]
    root: String,
}

#[tokio::main]
async fn main() {
    let _guard = configuration::setup_tracing_async("testrig".into());

    let cli = Cli::parse();
    let root = cli.root;

    tracing::info!(root, "Starting walker");

    let paths = WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| is_flac(e))
        .map(|e| e.into_path());

    let mut count = 0;
    for _path in paths {
        count += 1;
        if count % 1000 == 0 {
            tracing::info!(count, "Processed {} files", count);
        }
    }

    tracing::info!(count, "Done walking: {} flac files found", count);
}

fn is_flac(e: &walkdir::DirEntry) -> bool {
    e.file_type().is_file() && e.path().extension().unwrap_or_default() == "flac"
}
