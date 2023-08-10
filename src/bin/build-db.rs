use clap::Parser;
use rusqlite_migration::{Migrations, M};

use walkdir::WalkDir;
use wigglyair::database::{Database, DatabaseKind};
use wigglyair::metadata::TrackMetadata;
use wigglyair::{configuration, metadata};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[clap(short, long, help = "Limit the number of files to process")]
    limit: Option<usize>,

    #[clap(help = "Path to db file")]
    db: String,

    #[clap(help = "The root directory to scan")]
    root: String,
}

#[tokio::main]
async fn main() {
    configuration::setup_tracing("build-db".into());

    let cli = Cli::parse();

    let db = {
        let migrations = Migrations::new(vec![M::up(include_str!(
            "../../migrations/20230809235427-create-tracks.sql"
        ))]);
        let kind = DatabaseKind::parse(&cli.db);
        Database::connect(kind, migrations)
    };

    let repo = metadata::TrackMetadataRepository::new(db);

    let paths = WalkDir::new(cli.root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| is_flac(e))
        .map(|e| e.into_path())
        .take(cli.limit.unwrap_or(usize::MAX));

    for path in paths {
        let stat = metadata::stat_file(&path)
            .await
            .expect("Failed to get stat");
        let last_modified = metadata::last_modified(&stat).expect("Failed to get last modified");

        let track = repo
            .get_track_by_path(&path)
            .filter(|t| t.last_modified <= last_modified);

        if track.is_some() {
            tracing::info!("track exists: {}", path.display());
            continue;
        }

        let track = TrackMetadata::from_path_with_stat(&path, &stat)
            .await
            .expect("Failed to get track");
        tracing::info!("adding track: {}", track.debug_display());
        repo.add_track(&track);
    }
}

fn is_flac(e: &walkdir::DirEntry) -> bool {
    e.file_type().is_file() && e.path().extension().unwrap_or_default() == "flac"
}
