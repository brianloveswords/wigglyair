use std::{net::SocketAddr, path::Path};

use config::{Config, ConfigError};
use serde::Deserialize;
use tracing::subscriber::set_global_default;
use tracing_appender::non_blocking::{NonBlockingBuilder, WorkerGuard};
use tracing_bunyan_formatter::BunyanFormattingLayer;
use tracing_subscriber::{prelude::*, EnvFilter};

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

    pub fn addr(&self) -> SocketAddr {
        self.addr_string().parse().expect("Failed to parse address")
    }
}

pub fn setup_tracing_async(name: String) -> WorkerGuard {
    let logfile = format!("{}.log", &name);
    let logfile = Path::new(&logfile);
    let logdir = Path::new("logs");

    let file_appender = tracing_appender::rolling::daily(logdir, logfile);
    let (non_blocking, guard) = NonBlockingBuilder::default()
        .lossy(false)
        .finish(file_appender);
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt_layer = BunyanFormattingLayer::new(name, std::io::stdout.and(non_blocking));
    let subscriber = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer);

    set_global_default(subscriber).expect("Failed to set subscriber");
    guard
}

pub fn get_configuration() -> Result<Settings, ConfigError> {
    let settings = Config::builder()
        .add_source(config::File::new(
            "configuration.yml",
            config::FileFormat::Yaml,
        ))
        .build()?;
    settings.try_deserialize::<Settings>()
}
