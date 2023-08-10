use clap::Parser;
use rusqlite::{params, Connection};
use rusqlite_migration::{Migrations, M};
use wigglyair::configuration;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[clap(short, long, help = "Limit the number of files to process")]
    limit: Option<usize>,

    #[clap(help = "Path to db file")]
    db: String,

    #[clap(help = "The root directory to scan")]
    root: String,
}

fn main() {
    configuration::setup_tracing("testrig".into());

    let cli = Cli::parse();

    tracing::info!("ok: {:#?}", cli);

    let kind = DatabaseKind::parse(&cli.db);
    let mut db = Database::connect(kind);
}

struct Database {
    conn: Connection,
}

impl Database {
    #[tracing::instrument(name = "Database::connect")]
    fn connect(kind: DatabaseKind) -> Self {
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

        apply_migrations(&mut conn);

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
enum DatabaseKind {
    File(String),
    Memory,
}

impl DatabaseKind {
    fn parse(path: &str) -> Self {
        if path == ":memory:" {
            Self::Memory
        } else {
            Self::File(path.into())
        }
    }
}

#[tracing::instrument(skip(conn))]
fn apply_migrations(conn: &mut Connection) {
    let migrations = Migrations::new(vec![
        M::up("CREATE TABLE friend(name TEXT NOT NULL);"),
        // In the future, add more migrations here:
        //M::up("ALTER TABLE friend ADD COLUMN email TEXT;"),
    ]);

    migrations
        .to_latest(conn)
        .expect("Failed to apply migrations");
}
