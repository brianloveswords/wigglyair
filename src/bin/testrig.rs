use chrono::DateTime;
use clap::Parser;
use metaflac::block::VorbisComment;
use metaflac::Tag;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;
use walkdir::WalkDir;
use wigglyair::configuration;

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

    let cache_map = read_cached_metadata(&cache_path).expect("Failed to read cache file");

    for path in paths {
        let meta = TrackMetadata::read_from_path(&path, &cache_map)
            .expect(format!("Failed to read tags from {}", &path.display()).as_ref());
        // write ndjson to file
        let meta = serde_json::to_string(&meta).expect("Failed to serialize metadata");
        writeln!(cache_file, "{}", meta).expect("Failed to write to cache file");
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct TrackMetadata {
    pub path: PathBuf,
    pub last_modified: String,
    pub track_length: u32,
    pub album: String,
    pub artist: String,
    pub title: String,
    pub album_artist: String,
    pub track: u32,
}

fn stat_file(path: &PathBuf) -> Result<std::fs::Metadata, TrackMetadataError> {
    std::fs::metadata(path).map_err(|e| TrackMetadataError::IoFailed {
        path: path.clone(),
        error: e,
    })
}

fn last_modified(path: &PathBuf) -> Result<DateTime<chrono::Utc>, TrackMetadataError> {
    let stat = stat_file(path)?;
    let last_modified = stat.modified().map_err(|e| TrackMetadataError::IoFailed {
        path: path.clone(),
        error: e,
    })?;
    Ok(DateTime::from(last_modified))
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

#[tracing::instrument]
fn read_cached_metadata(path: &PathBuf) -> Result<FileMetadataMap, TrackMetadataError> {
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

impl TrackMetadata {
    #[tracing::instrument(name = "TrackMetadata::read_from_path")]
    fn read_from_path(path: &PathBuf, cache: &FileMetadataMap) -> Result<Self, TrackMetadataError> {
        let path_as_key = path.to_string_lossy().to_string();
        let last_modified = last_modified(path)?.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

        let cached_meta = cache
            .get(&path_as_key)
            .filter(|m| m.last_modified <= last_modified);

        if let Some(meta) = cached_meta {
            tracing::info!("Using cached metadata for {}", path.display());
            return Ok(meta.clone());
        }

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

        Ok(Self {
            path: path.to_path_buf(),
            last_modified,
            track_length,
            album,
            artist,
            title,
            album_artist,
            track,
        })
    }
}
