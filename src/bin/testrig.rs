use clap::Parser;
use std::io::{BufWriter, Write};
use std::path::Path;
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

fn is_flac(e: &walkdir::DirEntry) -> bool {
    e.file_type().is_file() && e.path().extension().unwrap_or_default() == "flac"
}

fn main() {
    configuration::setup_tracing("testrig".into());

    let cli = Cli::parse();

    // open a BufWriter wrapped config file
    let cache_path = Path::new(&cli.cache.unwrap_or("cache.json".into())).to_path_buf();
    let mut cache_file = {
        let cache_file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(&cache_path)
            .expect("Failed to open cache file");
        BufWriter::new(cache_file)
    };

    let paths = WalkDir::new(cli.root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| is_flac(e))
        .map(|e| e.into_path())
        .take(cli.limit.unwrap_or(usize::MAX));

    let cache_map = metadata::read_cached_metadata(&cache_path).expect("Failed to read cache file");

    for path in paths {
        let meta = TrackMetadata::read_from_path(&path, &cache_map)
            .expect(format!("Failed to read tags from {}", &path.display()).as_ref());
        // write ndjson to file
        let meta = serde_json::to_string(&meta).expect("Failed to serialize metadata");
        writeln!(cache_file, "{}", meta).expect("Failed to write to cache file");
    }
}
