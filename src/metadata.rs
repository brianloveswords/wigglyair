use chrono::{DateTime, SecondsFormat, Utc};
use metaflac::block::VorbisComment;
use metaflac::Tag;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::Metadata;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::database::Database;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct TrackMetadata {
    pub path: PathBuf,
    pub last_modified: String,
    pub file_size: u64,
    pub track_length: u32,
    pub album: String,
    pub artist: String,
    pub title: String,
    pub album_artist: String,
    pub track: u32,
}

impl TrackMetadata {
    pub fn full_album(&self) -> String {
        format!("{} - {}", self.album_artist, self.album)
    }

    pub fn padded_track(&self) -> String {
        format!("{:02}", self.track)
    }

    pub fn debug_display(&self) -> String {
        format!(
            "[{}]\n{}/{}/{} {} - {}",
            self.path.display().to_string(),
            self.album_artist,
            self.album,
            self.padded_track(),
            self.artist,
            self.title,
        )
    }
}

pub struct TrackMetadataRepository {
    db: Database,
}

impl TrackMetadataRepository {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    #[tracing::instrument(skip(self))]
    pub fn get_track_by_path(&self, path: &Path) -> Option<TrackMetadata> {
        let path = path.to_string_lossy();
        let mut stmt = self
            .db
            .conn
            .prepare(
                "
            SELECT
                path,
                last_modified,
                file_size,
                track_length,
                album,
                artist,
                title,
                album_artist,
                track
            FROM tracks
            WHERE path = ?1
            ",
            )
            .expect("Failed to prepare statement");

        let track = stmt
            .query_map(params![path], |row| track_from_row(row))
            .expect("Failed to query map")
            .next()
            .map(|t| t.expect("Failed to get track"));
        tracing::debug!("path {:?}", path);
        tracing::debug!("get_track_by_path: {:?}", track);
        track
    }

    #[tracing::instrument(skip(self))]
    pub fn add_track(&self, track: &TrackMetadata) {
        let mut stmt = self
            .db
            .conn
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
    }

    #[tracing::instrument(skip(self))]
    pub fn get_all_tracks(&self) -> Vec<TrackMetadata> {
        let mut stmt = self
            .db
            .conn
            .prepare(
                "
            SELECT
                path,
                last_modified,
                file_size,
                track_length,
                album,
                artist,
                title,
                album_artist,
                track
            FROM tracks
            ",
            )
            .expect("Failed to prepare statement");

        let track_iter = stmt
            .query_map([], track_from_row)
            .expect("Failed to query map");

        let mut tracks = Vec::new();
        for track in track_iter {
            tracks.push(track.expect("Failed to get track"));
        }
        tracks
    }
}

fn track_from_row(row: &rusqlite::Row<'_>) -> Result<TrackMetadata, rusqlite::Error> {
    let path_string: String = row.get(0)?;
    Ok(TrackMetadata {
        path: Path::new(&path_string).to_path_buf(),
        last_modified: row.get(1)?,
        file_size: row.get(2)?,
        track_length: row.get(3)?,
        album: row.get(4)?,
        artist: row.get(5)?,
        title: row.get(6)?,
        album_artist: row.get(7)?,
        track: row.get(8)?,
    })
}

#[derive(Error, Debug)]
pub enum TrackMetadataError {
    #[error("could not read from path")]
    ReadFailed(#[from] metaflac::Error),

    #[error("could not read from path")]
    IoFailed {
        path: PathBuf,
        error: std::io::Error,
    },

    #[error("invalid streaminfo")]
    InvalidStreamInfo { path: PathBuf },

    #[error("file missing comment")]
    MissingComment { path: PathBuf },

    #[error("file missing album")]
    MissingAlbum { path: PathBuf },

    #[error("file missing artist")]
    MissingArtist { path: PathBuf },

    #[error("file missing title")]
    MissingTitle { path: PathBuf },

    #[error("file missing album artist")]
    MissingAlbumArtist { path: PathBuf },

    #[error("file missing track")]
    MissingTrack { path: PathBuf },
}

pub type FileMetadataMap = BTreeMap<String, TrackMetadata>;

impl TrackMetadata {
    #[tracing::instrument]
    pub async fn from_path(path: &PathBuf) -> Result<Self, TrackMetadataError> {
        let stat = stat_file(path).await?;
        Self::from_path_with_stat(path, &stat).await
    }

    #[tracing::instrument]
    pub async fn from_path_with_stat(
        path: &PathBuf,
        stat: &std::fs::Metadata,
    ) -> Result<Self, TrackMetadataError> {
        let last_modified = last_modified(&stat).map_err(|e| TrackMetadataError::IoFailed {
            path: path.clone(),
            error: e,
        })?;

        let file_size: u64 = stat.len();
        let tag = read_tag_from_path(path)?;
        let comments =
            read_comments(&tag).ok_or(TrackMetadataError::MissingComment { path: path.clone() })?;

        let track_length = read_track_length(&tag)
            .ok_or(TrackMetadataError::InvalidStreamInfo { path: path.clone() })?;

        let album = comments
            .album()
            .and_then(|s| s.first().cloned())
            .ok_or(TrackMetadataError::MissingAlbum { path: path.clone() })?;

        let artist = comments
            .artist()
            .and_then(|s| s.first().cloned())
            .ok_or(TrackMetadataError::MissingArtist { path: path.clone() })?;

        let title = comments
            .title()
            .and_then(|s| s.first().cloned())
            .ok_or(TrackMetadataError::MissingTitle { path: path.clone() })?;

        let album_artist = comments
            .album_artist()
            .and_then(|s| s.first().cloned())
            .ok_or(TrackMetadataError::MissingAlbumArtist { path: path.clone() })?;

        let track = comments
            .track()
            .ok_or(TrackMetadataError::MissingTrack { path: path.clone() })?;

        let path = path.to_path_buf();
        Ok(Self {
            path,
            last_modified,
            file_size,
            track_length,
            album,
            artist,
            title,
            album_artist,
            track,
        })
    }
}

fn read_tag_from_path(path: &PathBuf) -> Result<Tag, TrackMetadataError> {
    Tag::read_from_path(&path).map_err(|e| TrackMetadataError::ReadFailed(e))
}

fn read_track_length(tag: &Tag) -> Option<u32> {
    let si = tag.get_streaminfo()?;
    // calculate length of track in seconds
    Some((si.total_samples / si.sample_rate as u64) as u32)
}

fn read_comments(tag: &Tag) -> Option<VorbisComment> {
    tag.vorbis_comments().cloned()
}

pub async fn stat_file(path: &PathBuf) -> Result<std::fs::Metadata, TrackMetadataError> {
    tokio::fs::metadata(path)
        .await
        .map_err(|e| TrackMetadataError::IoFailed {
            path: path.clone(),
            error: e,
        })
}

pub fn last_modified(stat: &Metadata) -> Result<String, io::Error> {
    stat.modified()
        .map(|t| DateTime::<Utc>::from(t).to_rfc3339_opts(SecondsFormat::Secs, true))
}
