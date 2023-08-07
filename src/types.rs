use crate::configuration::Settings;
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug)]
pub struct AppState {
    pub settings: Settings,
}

pub type SharedState = Arc<AppState>;

#[derive(Debug, Serialize)]
pub struct DebugResponse {
    pub paths: Vec<String>,
}
