use clap::Parser;
use std::path::Path;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use walkdir::WalkDir;
use wigglyair::metadata::TrackMetadata;
use wigglyair::{configuration, metadata};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[clap(short, long, help = "Limit the number of files to process")]
    limit: Option<usize>,

    #[clap(short, long, help = "Path to cache file")]
    cache: Option<String>,

    #[clap(help = "The root directory to scan")]
    root: String,
}

#[tokio::main]
async fn main() {
    configuration::setup_tracing("build-cache".into());

    let cli = Cli::parse();

    // open a BufWriter wrapped config file
    let cache_path = Path::new(&cli.cache.unwrap_or("cache.json".into())).to_path_buf();
    let cache_file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(&cache_path)
        .await
        .expect("Failed to open cache file");

    let cache_file = Arc::new(Mutex::new(cache_file));

    let cache_map =
        Arc::new(metadata::read_cached_metadata(&cache_path).expect("Failed to read cache file"));

    let tasks = WalkDir::new(cli.root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| is_flac(e))
        .map(|e| e.into_path())
        .take(cli.limit.unwrap_or(usize::MAX))
        .map(|path| {
            // set up a task that will attempt to read file metadata from the cache
            let cache_file = Arc::clone(&cache_file);
            let cache_map = Arc::clone(&cache_map);
            tokio::spawn(async move {
                let meta = TrackMetadata::read_from_path(&path, &cache_map)
                    .await
                    .expect(format!("Failed to read tags from {}", &path.display()).as_ref());

                let ndjson = {
                    let mut ndjson =
                        serde_json::to_string(&meta).expect("Failed to serialize metadata");
                    ndjson.push('\n');
                    ndjson
                };

                {
                    let mut cache_file = cache_file.lock().await;
                    cache_file
                        .write_all(ndjson.as_bytes())
                        .await
                        .expect("Failed to write to cache file");
                }
            })
        });

    for task in tasks {
        task.await.expect("Failed to join task");
    }
}

fn is_flac(e: &walkdir::DirEntry) -> bool {
    e.file_type().is_file() && e.path().extension().unwrap_or_default() == "flac"
}
