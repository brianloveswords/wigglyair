use tokio_rusqlite::Connection as AsyncConnection;

pub type Migrations<'a> = rusqlite_migration::Migrations<'a>;
pub type M<'a> = rusqlite_migration::M<'a>;

pub struct Database {
    pub conn: AsyncConnection,
}

impl Database {
    /// Connect to the database.
    ///
    /// # Panics
    ///
    /// Panics if the connection cannot be opened.
    pub async fn connect<'a>(kind: Kind) -> Self {
        let conn = match kind {
            Kind::File(path) => {
                tracing::info!("Opening database at {}", path);
                AsyncConnection::open(path)
                    .await
                    .expect("Failed to open connection")
            }
            Kind::Memory => {
                tracing::info!("Opening in-memory database");
                AsyncConnection::open_in_memory()
                    .await
                    .expect("Failed to open connection")
            }
        };

        conn.call(move |conn| {
            // see: https://cj.rs/blog/sqlite-pragma-cheatsheet-for-performance-and-consistency/
            conn.pragma_update(None, "journal_mode", "WAL")
        })
        .await
        .expect("Failed to set journal mode");

        Self { conn }
    }
}

#[derive(Debug)]
pub enum Kind {
    File(String),
    Memory,
}

impl Kind {
    pub fn parse(path: &str) -> Self {
        if path == ":memory:" {
            Self::Memory
        } else {
            Self::File(path.into())
        }
    }
}
