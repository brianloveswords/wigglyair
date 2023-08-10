use clap::Parser;
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

    println!("ok: {:#?}", cli)
}
