use axum::{
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use tracing::subscriber::set_global_default;
use tracing_bunyan_formatter::BunyanFormattingLayer;
use tracing_subscriber::{prelude::*, EnvFilter};

#[tokio::main]
async fn main() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt_layer = BunyanFormattingLayer::new("wigglyair".into(), std::io::stdout);

    let subscriber = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer);

    set_global_default(subscriber).expect("Failed to set subscriber");

    // build our application with a route
    let app = Router::new()
        // `GET /` goes to `root`
        .route("/", get(root))
        // `POST /users` goes to `create_user`
        .route("/users", post(create_user));

    // run our app with hyper `axum::Server` is a re-export of `hyper::Server`
    // so you can pass in any hyper settings here
    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    tracing::info!("listening on {addr}");
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

// basic handler that responds with a static string
#[tracing::instrument]
async fn root() -> &'static str {
    tracing::info!("handling request");
    "Hello, World!"
}

#[tracing::instrument]
async fn create_user(Json(payload): Json<CreateUser>) -> (StatusCode, Json<User>) {
    // insert application logic here
    let user = User {
        id: 42,
        username: payload.username,
    };

    tracing::info!("created user {:?}", user);

    // this will be converted into a JSON response
    // with a status code of `201 Created`
    (StatusCode::CREATED, Json(user))
}

// the input to our `create_user` handler
#[derive(Debug, Deserialize)]
struct CreateUser {
    username: String,
}

// the output to our `create_user` handler
#[derive(Debug, Serialize)]
struct User {
    id: u64,
    username: String,
}
