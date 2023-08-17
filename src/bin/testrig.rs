use std::{
    io::Write,
    str::FromStr,
    sync::{atomic::Ordering, Arc},
    thread,
    time::Duration,
};

use clap::Parser;
use std::sync::atomic::AtomicU8;
use tracing_unwrap::ResultExt;
use wigglyair::configuration;

#[derive(Debug)]
enum VolumeError {
    InvalidValue(u8),
    InvalidString(String),
}

#[derive(Debug)]
struct Volume(AtomicU8);

impl Volume {
    const MAX: u8 = 100;

    fn unsafe_from(initial: u8) -> Self {
        Self(AtomicU8::new(initial))
    }

    fn get(&self) -> u8 {
        self.0.load(Ordering::Acquire)
    }

    fn set(&self, value: u8) -> Result<(), VolumeError> {
        if value > Self::MAX {
            Err(VolumeError::InvalidValue(value))
        } else {
            self.0.store(value, Ordering::Release);
            Ok(())
        }
    }

    fn set_from_string(&self, value: String) -> Result<(), VolumeError> {
        let value: u8 = value
            .trim()
            .parse()
            .map_err(|_| VolumeError::InvalidString(value))?;
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

#[derive(Debug, Parser)]
struct Cli {
    #[clap(short, long, default_value = "100")]
    volume: u8,
}

#[tokio::main]
async fn main() {
    let _guard = configuration::setup_tracing_async("testrig".into());

    let cli = Cli::parse();
    let volume = Volume::try_from(cli.volume).expect_or_log("Could not parse volume");
    let volume = Arc::new(volume);

    let vol1 = volume.clone();
    let h1 = thread::spawn(move || loop {
        write!(std::io::stderr(), "> ").unwrap();

        let mut msg = String::new();
        if let Err(error) = std::io::stdin().read_line(&mut msg) {
            tracing::error!("error reading line: {}", error);
            break;
        }
        if msg == "" {
            break;
        }
        match vol1.set_from_string(msg) {
            Ok(()) => tracing::debug!(volume = vol1.get(), "set volume"),
            Err(error) => tracing::error!("error setting volume: {:?}", error),
        }
    });

    let h2 = thread::spawn(move || loop {
        let vol = volume.get();
        tracing::debug!(volume = vol, "reading volume");
        thread::sleep(Duration::from_secs(1));
    });

    h1.join().unwrap();
    h2.join().unwrap();
}
