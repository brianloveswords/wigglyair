use crate::configuration::Settings;
use itertools::FoldWhile::*;
use itertools::Itertools;
use serde::Serialize;
use std::path::PathBuf;
use std::{
    str::FromStr,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
};
use tinyaudio::OutputDeviceParameters;

#[derive(Debug)]
pub struct AppState {
    pub settings: Settings,
}

pub type SharedState = Arc<AppState>;

#[derive(Debug, Serialize)]
pub struct DebugResponse {
    pub paths: Vec<String>,
}

//
// Volume
//

#[derive(Debug)]
pub enum VolumeError {
    InvalidValue(u8),
    InvalidString(String),
}

#[derive(Debug)]
pub struct Volume(AtomicU8);

impl Volume {
    const MAX: u8 = 100;

    fn unsafe_from(initial: u8) -> Self {
        Self(AtomicU8::new(initial))
    }

    pub fn get(&self) -> u8 {
        self.0.load(Ordering::Acquire)
    }

    pub fn set(&self, value: u8) -> Result<(), VolumeError> {
        if value > Self::MAX {
            Err(VolumeError::InvalidValue(value))
        } else {
            self.0.store(value, Ordering::Release);
            Ok(())
        }
    }

    fn change(&self, value: i16) -> u8 {
        let mut ret = 0u8;
        self.0
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |prev| {
                let prev = prev as i16;
                let new = (prev + value as i16).clamp(0, 100);
                ret = new as u8;
                Some(new as u8)
            })
            .unwrap();
        ret
    }

    pub fn up(&self, value: u8) -> u8 {
        self.change(value as i16)
    }

    pub fn down(&self, value: u8) -> u8 {
        self.change(-(value as i16))
    }

    pub fn set_from_string(&self, value: &str) -> Result<(), VolumeError> {
        let value: u8 = value
            .trim()
            .parse()
            .map_err(|_| VolumeError::InvalidString(value.to_owned()))?;
        self.set(value)
    }
}

impl Default for Volume {
    fn default() -> Self {
        Self::unsafe_from(Self::MAX)
    }
}

impl TryFrom<u8> for Volume {
    type Error = VolumeError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if value > Self::MAX {
            Err(VolumeError::InvalidValue(value))
        } else {
            Ok(Self::unsafe_from(value))
        }
    }
}

impl TryFrom<String> for Volume {
    type Error = VolumeError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let value: u8 = value
            .trim()
            .parse()
            .map_err(|_| VolumeError::InvalidString(value))?;
        Self::try_from(value)
    }
}

impl FromStr for Volume {
    type Err = VolumeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value.to_string())
    }
}

//
// Audio Params
//

#[derive(Debug, Clone, Copy)]
pub struct AudioParams {
    pub channel_count: usize,
    pub sample_rate: usize,
}

impl AudioParams {
    const DEFAULT_AUDIO_BUFFER_FRAMES: u32 = 0;

    pub fn audio_buffer_frames(&self) -> u32 {
        Self::DEFAULT_AUDIO_BUFFER_FRAMES
    }

    pub fn channel_sample_count(&self) -> usize {
        self.sample_rate / 10
    }
}

impl From<AudioParams> for OutputDeviceParameters {
    fn from(other: AudioParams) -> Self {
        Self {
            channels_count: other.channel_count,
            sample_rate: other.sample_rate,
            channel_sample_count: other.channel_sample_count(),
        }
    }
}

//
// TrackList
//

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub path: PathBuf,
    pub samples: u64,
    pub channels: u8,
}

#[derive(Debug)]
pub struct TrackList {
    pub tracks: Vec<Track>,
    pub total_samples: u64,
}

impl TrackList {
    pub fn new() -> Self {
        Self {
            tracks: Vec::new(),
            total_samples: 0,
        }
    }

    pub fn add_track(&mut self, track: Track) {
        self.total_samples += track.samples;
        self.tracks.push(track);
    }

    pub fn add_tracks(&mut self, tracks: Vec<Track>) {
        self.total_samples += tracks.iter().map(|t| t.samples).sum::<u64>();
        self.tracks.extend(tracks);
    }

    pub fn find_playing(&self, current_sample: u64) -> &Track {
        let (found, _) = self
            .tracks
            .iter()
            .enumerate()
            .fold_while((0usize, 0u64), |(i, mut total), (j, track)| {
                total += track.samples;
                if total > current_sample {
                    Done((i, total))
                } else {
                    Continue((j, total))
                }
            })
            .into_inner();
        &self.tracks[found]
    }
}

impl Default for TrackList {
    fn default() -> Self {
        Self::new()
    }
}

impl Into<TrackList> for Vec<Track> {
    fn into(self) -> TrackList {
        let mut tl = TrackList::new();
        tl.add_tracks(self);
        tl
    }
}

//
// tests
//

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn test_volume_up_stays_below_100(amount: u8) {
            let v = Volume::default();
            let result = v.up(amount);
            prop_assert!(result <= 100);
        }

        #[test]
        fn test_volume_down_stays_below_100(amount: u8) {
            let v = Volume::default();
            let result = v.down(amount);
            prop_assert!(result <= 100);
        }
    }
}
