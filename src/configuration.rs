use std::net::SocketAddr;

use config::{Config, ConfigError};
use serde::Deserialize;
use tracing::subscriber::set_global_default;
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

pub fn setup_tracing(name: String) {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt_layer = BunyanFormattingLayer::new(name, std::io::stdout);

    let subscriber = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer);

    set_global_default(subscriber).expect("Failed to set subscriber");
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
