use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use tokio::task;
use wigglyair::configuration;
use wigglyair::database::AsyncDatabase;
use wigglyair::database::DatabaseKind;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[clap(help = "Path to db file")]
    db: String,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    configuration::setup_tracing("testrig".into());

    let db = AsyncDatabase::connect(DatabaseKind::parse(&cli.db)).await;
    let conn = Arc::new(db.conn);

    let conn1 = Arc::clone(&conn);
    let t1 = task::spawn(async move {
        loop {
            conn1
                .call(get_artist_count)
                .await
                .map_err(|e| tracing::error!("Query failed: {}", e))
                .unwrap();
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });

    let conn2 = Arc::clone(&conn);
    let t2 = task::spawn(async move {
        loop {
            conn2
                .call(get_album_count)
                .await
                .map_err(|e| tracing::error!("Query failed: {}", e))
                .unwrap();
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });

    let conn3 = conn.clone();
    let t3 = task::spawn(async move {
        loop {
            conn3
                .call(list_all_tracks)
                .await
                .map_err(|e| tracing::error!("Query failed: {}", e))
                .unwrap();
            tokio::time::sleep(Duration::from_secs(4)).await;
        }
    });

    t1.await.unwrap();
    t2.await.unwrap();
    t3.await.unwrap();
}

#[tracing::instrument(skip(conn))]
fn get_album_count(conn: &mut rusqlite::Connection) -> Result<u32, rusqlite::Error> {
    let query = "select count(distinct album) from tracks";
    let mut stmt = conn.prepare_cached(query)?;
    let total_albums: u32 = stmt
        .query_map([], |row| row.get(0))?
        .next()
        .unwrap()
        .unwrap();
    tracing::info!("Total albums: {}", total_albums);
    Ok(total_albums)
}

#[tracing::instrument(skip(conn))]
fn get_artist_count(conn: &mut rusqlite::Connection) -> Result<u32, rusqlite::Error> {
    let query = "select count(distinct artist) from tracks";
    let mut stmt = conn.prepare_cached(query)?;
    let total_artists: u32 = stmt
        .query_map([], |row| row.get(0))?
        .next()
        .unwrap()
        .unwrap();
    tracing::info!("Total artists: {}", total_artists);
    Ok(total_artists)
}

struct AlbumLine {
    album_artist: String,
    album: String,
    track_count: u32,
}

#[tracing::instrument(skip(conn))]
fn list_all_tracks(conn: &mut rusqlite::Connection) -> Result<(), rusqlite::Error> {
    let query = "
        select
            album_artist,
            album,
            count(1) as track_count
        from tracks
        group by 1,2
        order by last_modified desc
        limit 10
    ";
    let mut stmt = conn.prepare_cached(query)?;

    stmt.query_map([], |row| {
        let album_artist: String = row.get(0)?;
        let album: String = row.get(1)?;
        let track_count: u32 = row.get(2)?;
        Ok(AlbumLine {
            album_artist,
            album,
            track_count,
        })
    })?
    .filter_map(|r| match r {
        Ok(r) => Some(r),
        Err(e) => {
            tracing::error!("Error: {}", e);
            None
        }
    })
    .for_each(|line| {
        tracing::info!(
            "{} - {} ({})",
            line.album_artist,
            line.album,
            line.track_count
        );
    });
    Ok(())
}
