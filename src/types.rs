use crate::configuration::Settings;
use serde::Serialize;
use std::{
    str::FromStr,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
};

#[derive(Debug)]
pub struct AppState {
    pub settings: Settings,
}

pub type SharedState = Arc<AppState>;

#[derive(Debug, Serialize)]
pub struct DebugResponse {
    pub paths: Vec<String>,
}

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
