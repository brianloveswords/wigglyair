use clap::Parser;
use rusqlite_migration::{Migrations, M};
use wigglyair::{
    configuration,
    database::{Database, DatabaseKind},
};

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

    let db = {
        let migrations = Migrations::new(vec![
            M::up("CREATE TABLE friend(name TEXT NOT NULL);"),
            M::up("CREATE UNIQUE INDEX ux_friend_name ON friend(name);"),
        ]);
        let kind = DatabaseKind::parse(&cli.db);
        Database::connect(kind, migrations)
    };

    db.conn
        .execute(
            "insert or ignore into friend (name) values (?1)",
            (&"mother!",),
        )
        .expect("Failed to insert");
}
