use crate::types::DebugResponse;
use crate::types::SharedState;
use axum::extract::State;
use axum::Json;

// basic handler that responds with a static string
#[tracing::instrument]
pub async fn root() -> &'static [u8] {
    tracing::info!("handling request");
    "Hello, World!".as_bytes()
}

#[tracing::instrument(skip(state))]
pub async fn debug(State(state): State<SharedState>) -> Json<DebugResponse> {
    tracing::info!("state {:?}", state);
    let paths = state.settings.music.paths.clone();
    Json(DebugResponse { paths })
}
