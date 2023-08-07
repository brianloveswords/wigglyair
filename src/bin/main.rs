use axum::{
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;
use wigglyair::configuration;
use wigglyair::routes;

#[tokio::main]
async fn main() {
    configuration::setup_tracing("wigglyair".into());

    // build our application with a route
    let app = Router::new()
        .route("/", get(routes::root))
        .route("/debug", get(routes::debug));

    let addr = {
        let settings = configuration::get_configuration().expect("Failed to read configuration.");
        let host = settings.application.host;
        let port = settings.application.port;
        format!("{host}:{port}").parse::<SocketAddr>().unwrap()
    };

    tracing::info!("listening on {addr}");
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
