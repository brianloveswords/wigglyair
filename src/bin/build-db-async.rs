use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use rusqlite::params;
use rusqlite::Connection;
use rusqlite_migration::{Migrations, M};
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tokio::task;
use tokio_rusqlite::Connection as AsyncConnection;
use walkdir::WalkDir;
use wigglyair::{
    self, configuration,
    database::{AsyncDatabase, DatabaseKind},
    metadata::{self, TrackMetadata},
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

    let (tx, mut rx) = mpsc::channel::<AnalyzerMessage>(64);
    let conn = Arc::new(db.conn);

    let (writer_tx, mut writer_rx) = mpsc::channel::<WriterMessage>(64);
    let conn1 = Arc::clone(&conn);
    let t1 = task::spawn(async move {
        use AnalyzerMessage::*;
        tracing::info!("Starting analyzer");
        while let Some(msg) = rx.recv().await {
            match msg {
                AnalyzeFile(path) => analyze_file(path, &conn1, &writer_tx).await,
            }
        }
    });

    // walk dir, spit files
    let t2 = task::spawn(async move {
        tracing::info!("Starting walker");
        let paths = WalkDir::new(cli.root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| is_flac(e))
            .map(|e| e.into_path())
            .take(cli.limit.unwrap_or(usize::MAX));

        for path in paths {
            if let Err(e) = tx.send(AnalyzerMessage::AnalyzeFile(path)).await {
                tracing::error!("Failed to send path: {}", e)
            }
        }
    });

    let conn1 = Arc::clone(&conn);
    let t3 = task::spawn(async move {
        tracing::info!("Starting writer");
        let query = "
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
        ";
        while let Some(msg) = writer_rx.recv().await {
            match msg {
                WriterMessage::AddTrack(track) => {
                    let task = conn1.call(move |conn| {
                        tracing::info!("Adding track: {:?}", track);

                        let mut stmt = conn
                            .prepare_cached(query)
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

    t1.await.unwrap();
    t2.await.unwrap();
    t3.await.unwrap();
}

async fn analyze_file(path: PathBuf, conn: &AsyncConnection, tx: &Sender<WriterMessage>) {
    tracing::info!("Analyzing {}", path.display());

    let path = Arc::new(path);
    let stat = match metadata::stat_file(&path).await {
        Ok(stat) => stat,
        Err(err) => {
            tracing::error!("Failed to stat {}: {}", path.display(), err);
            return;
        }
    };

    let last_modified = metadata::last_modified(&stat).expect("Failed to get last modified");

    let is_up_to_date: bool = {
        let path = Arc::clone(&path);
        conn.call(move |conn| check_if_path_is_up_to_date(&path, &last_modified, conn))
            .await
            .unwrap()
    };

    if is_up_to_date {
        tracing::info!("{} is up to date", path.display());
        return;
    }

    let meta = match TrackMetadata::from_path_with_stat(&path, &stat).await {
        Ok(meta) => meta,
        Err(err) => {
            tracing::error!("Failed to get metadata for {}: {}", path.display(), err);
            return;
        }
    };

    tracing::info!("Got metadata for {}: {:?}", path.display(), meta);
    tx.send(WriterMessage::AddTrack(meta))
        .await
        .err()
        .map(|err| {
            tracing::error!("Failed to send metadata for {}: {}", path.display(), err);
        });
}

fn check_if_path_is_up_to_date(
    path: &PathBuf,
    last_modified: &String,
    conn: &Connection,
) -> Result<bool, rusqlite::Error> {
    let path = path.to_string_lossy();
    let mut stmt = conn.prepare_cached(
        "
            SELECT count(1) AS n
            FROM `tracks`
            WHERE 1=1
                AND `path` = ?1
                AND `last_modified` = ?2
        ",
    )?;
    let mut rows = stmt.query(params![path, last_modified])?;
    let n: i64 = rows.next()?.unwrap().get(0)?;
    Ok(n > 0)
}

fn is_flac(e: &walkdir::DirEntry) -> bool {
    e.file_type().is_file() && e.path().extension().unwrap_or_default() == "flac"
}
