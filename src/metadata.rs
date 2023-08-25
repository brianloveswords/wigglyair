use chrono::{DateTime, SecondsFormat, Utc};
use metaflac::block::{StreamInfo, VorbisComment};
use metaflac::Tag;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::Metadata;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct TrackMetadata {
    pub path: PathBuf,
    pub last_modified: String,
    pub file_size: u64,
    pub sample_rate: u32,
    pub total_samples: u64,
    pub length_secs: u32,
    pub channels: u8,
    pub max_block_size: u16,
    pub album: String,
    pub artist: String,
    pub title: String,
    pub album_artist: String,
    pub track: u32,
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
    pub async fn from_path(path: PathBuf) -> Result<Self, TrackMetadataError> {
        let stat = stat_file(&path).await?;
        Self::from_path_with_stat(path, &stat).await
    }

    pub async fn from_path_with_stat(
        path: PathBuf,
        stat: &std::fs::Metadata,
    ) -> Result<Self, TrackMetadataError> {
        let last_modified = last_modified(stat).map_err(|e| TrackMetadataError::IoFailed {
            path: path.to_path_buf(),
            error: e,
        })?;

        let file_size: u64 = stat.len();
        let tag = read_tag_from_path(&path)?;
        let streaminfo = tag
            .get_streaminfo()
            .ok_or(TrackMetadataError::InvalidStreamInfo { path: path.clone() })?;

        let length_secs = calc_length_secs(streaminfo);
        let max_block_size = streaminfo.max_block_size;
        let total_samples = streaminfo.total_samples;
        let sample_rate = streaminfo.sample_rate;
        let channels = streaminfo.num_channels;

        let comments =
            read_comments(&tag).ok_or(TrackMetadataError::MissingComment { path: path.clone() })?;

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
            sample_rate,
            total_samples,
            length_secs,
            channels,
            max_block_size,
            album,
            artist,
            title,
            album_artist,
            track,
        })
    }
}

fn read_tag_from_path(path: &Path) -> Result<Tag, TrackMetadataError> {
    Tag::read_from_path(path).map_err(TrackMetadataError::ReadFailed)
}

fn calc_length_secs(si: &StreamInfo) -> u32 {
    // calculate length of track in seconds
    (si.total_samples / si.sample_rate as u64) as u32
}

fn read_comments(tag: &Tag) -> Option<VorbisComment> {
    tag.vorbis_comments().cloned()
}

pub async fn stat_file(path: &Path) -> Result<std::fs::Metadata, TrackMetadataError> {
    tokio::fs::metadata(path)
        .await
        .map_err(|e| TrackMetadataError::IoFailed {
            path: path.to_path_buf(),
            error: e,
        })
}

pub fn last_modified(stat: &Metadata) -> Result<String, io::Error> {
    stat.modified()
        .map(|t| DateTime::<Utc>::from(t).to_rfc3339_opts(SecondsFormat::Secs, true))
}
