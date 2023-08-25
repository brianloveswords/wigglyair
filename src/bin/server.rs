use std::sync::Arc;

use axum::{routing::get, Router};
use wigglyair::{configuration, routes, types::AppState};

#[tokio::main]
async fn main() {
    let _guard = configuration::setup_tracing_async("wigglyair".into());
    let settings =
        configuration::from_file("configuration.yml").expect("Failed to read configuration.");
    let addr = settings.server.addr();

    let state = AppState { settings };

    // build our application with a route
    let app = Router::new()
        .route("/", get(routes::root))
        .route("/debug", get(routes::debug))
        .with_state(Arc::new(state));

    tracing::info!("listening on {addr}");
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
