use std::path::PathBuf;

use clap::Parser;
use rusqlite::params;
use rusqlite_migration::{Migrations, M};
use tokio::sync::mpsc::{self, UnboundedSender};
use walkdir::WalkDir;
use wigglyair::{
    configuration,
    database::{AsyncDatabase, DatabaseKind},
    metadata::TrackMetadata,
};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[clap(short, long, help = "Limit the number of files to process")]
    limit: Option<usize>,

    #[clap(help = "Path to db file")]
    db: String,

    #[clap(help = "The root directory to scan")]
    root: String,
}

enum AnalyzerMessage {
    AnalyzeFile(PathBuf),
}

enum WriterMessage {
    AddTrack(TrackMetadata),
}

#[tokio::main]
async fn main() {
    configuration::setup_tracing("build-db-async".into());

    let cli = Cli::parse();

    // set up the async database connection
    let db = {
        let db = AsyncDatabase::connect(DatabaseKind::parse(&cli.db)).await;
        db.conn
            .call(|conn| {
                Migrations::new(vec![M::up(include_str!(
                    "../../migrations/20230809235427-create-tracks.sql"
                ))])
                .to_latest(conn)
                .expect("Failed to run migrations");
                Ok(())
            })
            .await
            .expect("Failed to run migrations");
        db
    };

    // what does the analyzer look like?
    // we need to set up a channel for the walker to spit pathbufs into
    // we need the reader that's gonna loop on that channel and analyze the files
    // we can use tokio to spawn tasks that will do the work

    let (tx, mut rx) = mpsc::unbounded_channel::<AnalyzerMessage>();
    let (writer_tx, mut writer_rx) = mpsc::unbounded_channel::<WriterMessage>();

    // walk dir, spit files
    tokio::spawn(async move {
        let paths = WalkDir::new(cli.root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| is_flac(e))
            .map(|e| e.into_path())
            .take(cli.limit.unwrap_or(usize::MAX));

        paths.for_each(|path| match tx.send(AnalyzerMessage::AnalyzeFile(path)) {
            Err(err) => {
                tracing::error!("Failed to send path: {}", err)
            }
            _ => {}
        });
    });

    tokio::task::spawn(async move {
        while let Some(msg) = writer_rx.recv().await {
            match msg {
                WriterMessage::AddTrack(track) => {
                    let task = db.conn.call(move |conn| {
                        tracing::info!("Adding track: {:?}", track);
                        let mut stmt = conn
                            .prepare(
                                "
                            INSERT INTO tracks (
                                path,
                                last_modified,
                                file_size,
                                track_length,
                                album,
                                artist,
                                title,
                                album_artist,
                                track
                            )
                            VALUES (
                                ?1,
                                ?2,
                                ?3,
                                ?4,
                                ?5,
                                ?6,
                                ?7,
                                ?8,
                                ?9
                            )
                            ",
                            )
                            .expect("Failed to prepare statement");

                        stmt.execute(params![
                            track.path.to_str().unwrap(),
                            track.last_modified,
                            track.file_size,
                            track.track_length,
                            track.album,
                            track.artist,
                            track.title,
                            track.album_artist,
                            track.track,
                        ])
                        .expect("Failed to execute statement");
                        Ok(())
                    });
                    task.await
                        .err()
                        .map(|err| tracing::error!("Failed to add track: {}", err));
                }
            }
        }
    });

    while let Some(msg) = rx.recv().await {
        match msg {
            AnalyzerMessage::AnalyzeFile(path) => analyze_file(path, &writer_tx).await,
        }
    }
}

async fn analyze_file(path: PathBuf, tx: &UnboundedSender<WriterMessage>) {
    let meta = match TrackMetadata::from_path(&path).await {
        Ok(meta) => meta,
        Err(err) => {
            tracing::error!("Failed to get metadata for {}: {}", path.display(), err);
            return;
        }
    };

    tracing::info!("Got metadata for {}: {:?}", path.display(), meta);
    tx.send(WriterMessage::AddTrack(meta)).err().map(|err| {
        tracing::error!("Failed to send metadata for {}: {}", path.display(), err);
    });
}

fn is_flac(e: &walkdir::DirEntry) -> bool {
    e.file_type().is_file() && e.path().extension().unwrap_or_default() == "flac"
}
