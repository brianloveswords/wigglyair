use rusqlite::Connection;
use tokio_rusqlite::Connection as AsyncConnection;

pub type Migrations<'a> = rusqlite_migration::Migrations<'a>;
pub type M<'a> = rusqlite_migration::M<'a>;

pub struct Database {
    pub conn: Connection,
}

impl Database {
    #[tracing::instrument(name = "Database::connect")]
    pub fn connect(kind: DatabaseKind, migrations: Migrations) -> Self {
        let mut conn = match kind {
            DatabaseKind::File(path) => {
                tracing::info!("Opening database at {}", path);
                Connection::open(path).expect("Failed to open connection")
            }
            DatabaseKind::Memory => {
                tracing::info!("Opening in-memory database");
                Connection::open_in_memory().expect("Failed to open connection")
            }
        };

        // see: https://cj.rs/blog/sqlite-pragma-cheatsheet-for-performance-and-consistency/
        conn.pragma_update(None, "journal_mode", &"WAL")
            .expect("Failed to set journal mode");

        migrations
            .to_latest(&mut conn)
            .expect("Failed to apply migrations");

        Self { conn }
    }
}

pub struct AsyncDatabase {
    pub conn: AsyncConnection,
}

impl AsyncDatabase {
    pub async fn connect<'a>(kind: DatabaseKind) -> Self {
        let conn = match kind {
            DatabaseKind::File(path) => {
                tracing::info!("Opening database at {}", path);
                AsyncConnection::open(path)
                    .await
                    .expect("Failed to open connection")
            }
            DatabaseKind::Memory => {
                tracing::info!("Opening in-memory database");
                AsyncConnection::open_in_memory()
                    .await
                    .expect("Failed to open connection")
            }
        };

        conn.call(move |conn| {
            // see: https://cj.rs/blog/sqlite-pragma-cheatsheet-for-performance-and-consistency/
            conn.pragma_update(None, "journal_mode", &"WAL")
        })
        .await
        .expect("Failed to set journal mode");

        Self { conn }
    }
}

impl Drop for Database {
    #[tracing::instrument(skip(self))]
    fn drop(&mut self) {
        let conn = &self.conn;
        conn.pragma_update(None, "analysis_limit", &400)
            .expect("Failed to set analysis limit");
        conn.pragma_update(None, "optimize", "")
            .expect("Failed to optimize");
    }
}

#[derive(Debug)]
pub enum DatabaseKind {
    File(String),
    Memory,
}

impl DatabaseKind {
    pub fn parse(path: &str) -> Self {
        if path == ":memory:" {
            Self::Memory
        } else {
            Self::File(path.into())
        }
    }
}
