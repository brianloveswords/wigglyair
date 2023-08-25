use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

use config::{Config, ConfigError};
use directories::ProjectDirs;
use serde::Deserialize;
use tracing::subscriber::set_global_default;
use tracing_appender::non_blocking::{NonBlockingBuilder, WorkerGuard};
use tracing_bunyan_formatter::BunyanFormattingLayer;
use tracing_subscriber::{prelude::*, EnvFilter};
use tracing_unwrap::*;

#[derive(Debug, Deserialize)]
pub struct Settings {
    pub server: ServerSettings,
    pub music: MusicSettings,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MusicSettings {
    pub paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ServerSettings {
    pub port: u16,
    pub host: String,
}
impl ServerSettings {
    pub fn addr_string(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Get the address as a `SocketAddr`
    ///
    /// # Panics
    ///
    /// Panics if the address cannot be parsed
    pub fn addr(&self) -> SocketAddr {
        self.addr_string().parse().expect("Failed to parse address")
    }
}

/// Setup tracing
///
/// This will setup tracing to log to a file in the log directory
/// based on the name of the application.
///
/// # Panics
///
/// Panics if the subscriber could not be set
///
/// # See Also
///
/// - [`get_log_dir`]
pub fn setup_tracing_async(name: String) -> WorkerGuard {
    let log_file = Path::new("log");
    let log_dir = get_log_dir(&name);

    let file_appender = tracing_appender::rolling::daily(log_dir, log_file);
    let (non_blocking, guard) = NonBlockingBuilder::default()
        .lossy(false)
        .finish(file_appender);
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt_layer = BunyanFormattingLayer::new(name, non_blocking);
    let subscriber = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer);

    set_global_default(subscriber).expect("Failed to set subscriber");
    guard
}

/// Get the log directory for the application
///
/// # Panics
///
/// Panics if the project directories cannot be retrieved
pub fn get_log_dir(name: &str) -> PathBuf {
    let project_dirs =
        ProjectDirs::from("com", "wigglyair", name).expect_or_log("Failed to get project dirs");

    // state_dir only exists on Linux, so we'll fall back to `{cache_dir}/logs`.
    // went back and forth on whether this belongs in data or cache, but since
    // logs are not required for the app to function, cache feels right.
    project_dirs.state_dir().map_or_else(
        || {
            let mut cache_dir = project_dirs.cache_dir().to_path_buf();
            cache_dir.push("logs");
            cache_dir
        },
        Path::to_path_buf,
    )
}

/// Read a configuration file and deserialize it into a Settings struct.
///
/// # Errors
///
/// Returns a `ConfigError` if the file cannot be read or deserialized.
pub fn from_file(file: &str) -> Result<Settings, ConfigError> {
    let settings = Config::builder()
        .add_source(config::File::new(file, config::FileFormat::Yaml))
        .build()?;
    settings.try_deserialize::<Settings>()
}
