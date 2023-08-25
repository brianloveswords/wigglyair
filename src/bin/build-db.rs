use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use crossbeam::channel;
use futures::future;
use rusqlite::params;
use rusqlite::Connection;
use rusqlite_migration::{Migrations, M};
use tokio::task;
use tokio_rusqlite::Connection as AsyncConnection;
use tracing_unwrap::*;
use walkdir::DirEntry;
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

    #[clap(long, help = "Filter files by pattern")]
    filter: Option<String>,

    #[clap(help = "Path to db file")]
    db: String,

    #[clap(help = "The root directory to scan")]
    root: String,
}

#[derive(Debug)]
enum AnalyzerMessage {
    AnalyzeFile(PathBuf),
}

#[derive(Debug)]
enum WriterMessage {
    AddTrack(TrackMetadata),
}

#[tokio::main]
async fn main() {
    // flush logs when the this guard leaves scope, hopefully at the end of the program
    let _guard = configuration::setup_tracing_async("build-db".into());

    let cli = Cli::parse();
    let db_path = cli.db;

    // set up the async database connection
    let db = {
        let db = AsyncDatabase::connect(DatabaseKind::parse(&db_path)).await;
        db.conn
            .call(|conn| {
                Migrations::new(vec![M::up(include_str!(
                    "../../migrations/20230809235427-create-tracks.sql"
                ))])
                .to_latest(conn)
                .unwrap_or_log();
                Ok(())
            })
            .await
            .unwrap_or_log();
        db
    };
    let conn = Arc::new(db.conn);

    let (analyzer_tx, analyzer_rx) = channel::unbounded::<AnalyzerMessage>();
    let (writer_tx, writer_rx) = channel::unbounded::<WriterMessage>();

    let analyzer_tasks = (0..4).map(|id| {
        let conn = Arc::clone(&conn);
        let analyzer_rx = analyzer_rx.clone();
        let writer_tx = writer_tx.clone();
        task::spawn(async move {
            use AnalyzerMessage::*;
            tracing::info!(id, "Starting analyzer");

            loop {
                let msg_opt = analyzer_rx.recv();
                match msg_opt {
                    Ok(msg) => match msg {
                        AnalyzeFile(path) => {
                            analyze_file(id, path, &conn, &writer_tx).await;
                        }
                    },
                    Err(_) => break,
                }
            }
            tracing::info!(id, "Finished analyzer");
        })
    });

    let conn1 = Arc::clone(&conn);
    task::spawn(async move {
        tracing::info!(db_path, "Starting writer");
        let query = "
            INSERT INTO tracks (
                path,
                last_modified,
                file_size,
                sample_rate,
                total_samples,
                length_secs,
                channels,
                max_block_size,
                album,
                artist,
                title,
                album_artist,
                track
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        ";
        while let Ok(msg) = writer_rx.recv() {
            match msg {
                WriterMessage::AddTrack(track) => {
                    conn1
                        .call(move |conn| {
                            tracing::debug!(?track, "Adding track");

                            let mut stmt = conn
                                .prepare_cached(query)
                                .expect_or_log("Failed to prepare statement");

                            stmt.execute(params![
                                track.path.to_str().unwrap_or_log(),
                                track.last_modified,
                                track.file_size,
                                track.sample_rate,
                                track.total_samples,
                                track.length_secs,
                                track.channels,
                                track.max_block_size,
                                track.album,
                                track.artist,
                                track.title,
                                track.album_artist,
                                track.track,
                            ])
                            .expect_or_log("Failed to execute statement");
                            Ok(())
                        })
                        .await
                        .expect_or_log("Failed to add track");
                }
            }
        }
        tracing::info!(db_path, "Finished writer");
    });

    let walker_task = tokio::spawn(async move {
        let root = cli.root;

        tracing::info!(%root, "Starting walker");

        let path_filter = path_filter_from_opt(cli.filter);
        let paths = WalkDir::new(&root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(is_flac)
            .filter(path_filter)
            .map(|e| e.into_path())
            .take(cli.limit.unwrap_or(usize::MAX));

        for path in paths {
            analyzer_tx
                .send(AnalyzerMessage::AnalyzeFile(path))
                .expect_or_log("Failed to send path for analysis")
        }

        tracing::info!(%root, "Finished walking");
    });

    // join everything to make sure we don't drop the channels before they're done
    // don't join the writer, though—we want that to drop once we're done analyzing
    // otherwise we'll hang on to the writer too long.
    let mut all_tasks = analyzer_tasks.collect::<Vec<_>>();
    all_tasks.push(walker_task);

    for result in future::join_all(all_tasks).await {
        result.expect_or_log("Failed to join task");
    }
}

fn path_filter_from_opt(filter: Option<String>) -> Box<dyn Fn(&DirEntry) -> bool + Send> {
    match filter {
        Some(filter) => Box::new(move |e: &DirEntry| {
            fuzzy_match_string(&filter, e.path().to_string_lossy().to_string().as_str())
        }),
        None => Box::new(|_| true),
    }
}

fn fuzzy_match_string(needle: &str, haystack: &str) -> bool {
    let needle = needle.to_lowercase();
    let haystack = haystack.to_lowercase();

    let needle_words: Vec<Option<usize>> = needle
        .split_whitespace()
        .map(|word| haystack.find(word))
        .collect();

    let mut last_index: Option<usize> = None;
    for word in needle_words {
        match (word, last_index) {
            (None, _) => return false,
            (Some(index), Some(li)) if index < li => return false,
            (Some(index), _) => last_index = Some(index),
        }
    }
    true
}

async fn analyze_file(
    id: u32,
    path: PathBuf,
    conn: &AsyncConnection,
    tx: &channel::Sender<WriterMessage>,
) {
    tracing::debug!(id, path = %path.display(), "Analyzing file");

    let path = Arc::new(path);
    let stat = match metadata::stat_file(&path).await {
        Ok(stat) => stat,
        Err(err) => {
            tracing::error!(id, %err, path = %path.display(), "Failed to stat");
            return;
        }
    };

    let last_modified = metadata::last_modified(&stat).expect_or_log("Failed to get last modified");

    let is_up_to_date: bool = {
        let path = Arc::clone(&path);
        conn.call(move |conn| check_path_is_up_to_date(&path, &last_modified, conn))
            .await
            .unwrap_or_log()
    };

    if is_up_to_date {
        tracing::debug!(id, path = %path.display(), "Up to date");
        return;
    }

    let meta = match TrackMetadata::from_path_with_stat(path.to_path_buf(), &stat).await {
        Ok(meta) => meta,
        Err(err) => {
            tracing::error!(
                id,
                %err,
                path = %path.display(),
                "Failed to get metadata",
            );
            return;
        }
    };

    tracing::debug!(id, ?meta, path = %path.display(), "Got metadata");
    if let Some(error) = tx.send(WriterMessage::AddTrack(meta)).err() {
        tracing::error!(id, %error, path = %path.display(), "Failed to send metadata");
    }
}

fn check_path_is_up_to_date(
    path: &Path,
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
    let n: i64 = rows.next()?.unwrap_or_log().get(0)?;
    Ok(n > 0)
}

fn is_flac(e: &walkdir::DirEntry) -> bool {
    e.file_type().is_file() && e.path().extension().unwrap_or_default() == "flac"
}
