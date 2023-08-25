use chrono::{DateTime, SecondsFormat, Utc};
use metaflac::block::{StreamInfo, VorbisComment};
use metaflac::Tag;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::Metadata;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing_unwrap::ResultExt;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Track {
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

pub type FileMetadataMap = BTreeMap<String, Track>;

impl Track {
    /// Create a `TrackMetadata` from a path
    ///
    /// # Errors
    ///
    /// This function will return an error if the metadata cannot be read
    pub async fn from_path(path: PathBuf) -> Result<Self, TrackMetadataError> {
        let stat = stat_file(&path).await?;
        Self::from_path_with_stat(&path, &stat)
    }

    /// Create a `TrackMetadata` from a path and a stat
    ///
    /// # Errors
    ///
    /// This function will return an error if the metadata cannot be read
    /// from the file.
    pub fn from_path_with_stat(
        path: &Path,
        stat: &std::fs::Metadata,
    ) -> Result<Self, TrackMetadataError> {
        let last_modified = last_modified(stat).map_err(|e| TrackMetadataError::IoFailed {
            path: path.to_path_buf(),
            error: e,
        })?;

        let file_size: u64 = stat.len();
        let tag = read_tag_from_path(path)?;
        let streaminfo = tag
            .get_streaminfo()
            .ok_or(TrackMetadataError::InvalidStreamInfo {
                path: path.to_path_buf(),
            })?;

        let length_secs = calc_length_secs(streaminfo);
        let max_block_size = streaminfo.max_block_size;
        let total_samples = streaminfo.total_samples;
        let sample_rate = streaminfo.sample_rate;
        let channels = streaminfo.num_channels;

        let comments = read_comments(&tag).ok_or(TrackMetadataError::MissingComment {
            path: path.to_path_buf(),
        })?;

        let album = comments.album().and_then(|s| s.first().cloned()).ok_or(
            TrackMetadataError::MissingAlbum {
                path: path.to_path_buf(),
            },
        )?;

        let artist = comments.artist().and_then(|s| s.first().cloned()).ok_or(
            TrackMetadataError::MissingArtist {
                path: path.to_path_buf(),
            },
        )?;

        let title = comments.title().and_then(|s| s.first().cloned()).ok_or(
            TrackMetadataError::MissingTitle {
                path: path.to_path_buf(),
            },
        )?;

        let album_artist = comments
            .album_artist()
            .and_then(|s| s.first().cloned())
            .ok_or(TrackMetadataError::MissingAlbumArtist {
                path: path.to_path_buf(),
            })?;

        let track = comments.track().ok_or(TrackMetadataError::MissingTrack {
            path: path.to_path_buf(),
        })?;

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
    u32::try_from(si.total_samples / u64::from(si.sample_rate))
        .expect_or_log("calc_length_secs: overflow")
}

fn read_comments(tag: &Tag) -> Option<VorbisComment> {
    tag.vorbis_comments().cloned()
}

/// Returns the metadata for a file
///
/// # Errors
///
/// Returns an error if the file's metadata cannot be read
pub async fn stat_file(path: &Path) -> Result<std::fs::Metadata, TrackMetadataError> {
    tokio::fs::metadata(path)
        .await
        .map_err(|e| TrackMetadataError::IoFailed {
            path: path.to_path_buf(),
            error: e,
        })
}

/// Returns the last modified time of a file as an RFC3339 formatted string
///
/// # Errors
///
/// Returns an error if the file's last modified time cannot be read
pub fn last_modified(stat: &Metadata) -> Result<String, io::Error> {
    stat.modified()
        .map(|t| DateTime::<Utc>::from(t).to_rfc3339_opts(SecondsFormat::Secs, true))
}
