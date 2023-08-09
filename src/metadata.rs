use chrono::{DateTime, SecondsFormat, Utc};
use metaflac::block::VorbisComment;
use metaflac::Tag;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::Metadata;
use std::io;
use std::path::PathBuf;
use thiserror::Error;

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
    #[tracing::instrument(name = "TrackMetadata::read_from_path")]
    pub async fn read_from_path(
        path: &PathBuf,
        cache: &FileMetadataMap,
    ) -> Result<Self, TrackMetadataError> {
        let stat = stat_file(path).await?;
        let last_modified = last_modified(&stat).map_err(|e| TrackMetadataError::IoFailed {
            path: path.clone(),
            error: e,
        })?;

        let cached_meta = {
            let path_as_key = path.to_string_lossy().to_string();
            cache
                .get(&path_as_key)
                .filter(|m| m.last_modified <= last_modified)
        };

        if let Some(meta) = cached_meta {
            tracing::info!("Using cached metadata for {}", path.display());
            return Ok(meta.clone());
        }

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

#[tracing::instrument]
pub fn read_cached_metadata(path: &PathBuf) -> Result<FileMetadataMap, TrackMetadataError> {
    if !path.exists() {
        tracing::info!("No cache file found, starting from scratch");
        return Ok(FileMetadataMap::new());
    }

    let contents = std::fs::read_to_string(&path).expect("Failed to read cache file");

    let mut map = FileMetadataMap::new();
    // split cache contents on newlines and parse each line as a TrackMetadata,
    // then insert it into the cache map
    contents.lines().for_each(|line| {
        if line.trim().is_empty() {
            return;
        }

        let meta: TrackMetadata =
            serde_json::from_str(line).expect("Failed to deserialize cache file");

        map.insert(meta.path.to_string_lossy().into(), meta);
    });

    Ok(map)
}

#[tracing::instrument(name = "Tag::read_from_path")]
fn read_tag_from_path(path: &PathBuf) -> Result<Tag, TrackMetadataError> {
    Tag::read_from_path(&path).map_err(|e| TrackMetadataError::ReadFailed(e))
}

#[tracing::instrument(skip(tag))]
fn read_track_length(tag: &Tag) -> Option<u32> {
    let si = tag.get_streaminfo()?;
    // calculate length of track in seconds
    Some((si.total_samples / si.sample_rate as u64) as u32)
}

#[tracing::instrument(skip(tag))]
fn read_comments(tag: &Tag) -> Option<VorbisComment> {
    tag.vorbis_comments().cloned()
}

async fn stat_file(path: &PathBuf) -> Result<std::fs::Metadata, TrackMetadataError> {
    tokio::fs::metadata(path)
        .await
        .map_err(|e| TrackMetadataError::IoFailed {
            path: path.clone(),
            error: e,
        })
}

fn last_modified(stat: &Metadata) -> Result<String, io::Error> {
    stat.modified()
        .map(|t| DateTime::<Utc>::from(t).to_rfc3339_opts(SecondsFormat::Secs, true))
}
