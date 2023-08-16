use clap::Parser;
use std::sync::atomic::{AtomicU32, Ordering};
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

    let count = AtomicU32::new(0);
    WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| is_flac(e))
        .map(|e| e.into_path())
        .for_each(|_| {
            let old_count = count.fetch_add(1, Ordering::SeqCst);
            let new_count = old_count + 1;
            if new_count % 1000 == 0 {
                tracing::info!(count = new_count, "Processed {} files", new_count);
            }
        });

    let total_count = count.load(Ordering::SeqCst);
    tracing::info!(
        total_count,
        "Done walking: {} flac files found",
        total_count
    );
}

fn is_flac(e: &walkdir::DirEntry) -> bool {
    e.file_type().is_file() && e.path().extension().unwrap_or_default() == "flac"
}
