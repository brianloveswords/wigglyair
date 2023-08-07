use crate::configuration::Settings;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug)]
pub struct AppState {
    pub settings: Settings,
}

pub type SharedState = Arc<AppState>;

// the input to our `create_user` handler
#[derive(Debug, Deserialize)]
pub struct CreateUser {
    pub username: String,
}

// the output to our `create_user` handler
#[derive(Debug, Serialize)]
pub struct User {
    pub id: u64,
    pub username: String,
}

#[derive(Debug, Serialize)]
pub struct DebugResponse {
    pub paths: Vec<String>,
}
