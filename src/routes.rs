use crate::types::CreateUser;
use crate::types::User;
use axum::http::StatusCode;
use axum::Json;

// basic handler that responds with a static string
#[tracing::instrument]
pub async fn root() -> &'static str {
    tracing::info!("handling request");
    "Hello, World!"
}

#[tracing::instrument]
pub async fn debug() -> &'static str {
    tracing::info!("handling request");
    "Hello, Debug!"
}

#[tracing::instrument]
pub async fn create_user(Json(payload): Json<CreateUser>) -> (StatusCode, Json<User>) {
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
