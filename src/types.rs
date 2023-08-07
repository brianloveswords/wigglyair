use serde::{Deserialize, Serialize};

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
