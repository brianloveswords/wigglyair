use std::path::{Path, PathBuf};

use itertools::Itertools;
use walkdir::WalkDir;

/// Returns true if the path exists and is a supported audio file.
///
/// Currently only flac is supported.
pub fn is_supported_audio_file<P: AsRef<Path>>(p: P) -> bool {
    let p = p.as_ref();
    p.exists() && p.extension().unwrap_or_default() == "flac"
}

/// Walk directories and filter down to only flac files
///
/// When an entry is a directory, it will be walked and all audio files
/// will be included. When it's a file, it will be included if it's audio.
///
/// The returned paths will be canonicalized.
pub fn only_audio_files(filenames: Vec<String>) -> Vec<PathBuf> {
    filenames
        .iter()
        .map(Path::new)
        .flat_map(|p| {
            if p.is_dir() {
                WalkDir::new(p)
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().is_file())
                    .map(|e| e.path().to_owned())
                    .collect_vec()
            } else {
                vec![p.to_owned()]
            }
        })
        .filter(|p| is_supported_audio_file(p))
        .map(|p| p.canonicalize().unwrap())
        .collect_vec()
}
