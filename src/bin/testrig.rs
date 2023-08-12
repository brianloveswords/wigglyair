use clap::Parser;
use tokio::sync::mpsc;
use wigglyair::configuration;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[clap(help = "Path to db file")]
    db: String,
}

#[tokio::main]
async fn main() {
    configuration::setup_tracing("testrig".into());

    let (tx, mut rx) = mpsc::channel(100);

    let t1 = tokio::spawn(async move {
        loop {
            let msg = rx.recv().await;
            tracing::info!("Received message: {:?}", msg);
        }
    });

    let t2 = tokio::spawn(async move {
        loop {
            tx.send("Hello".to_string()).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });

    t1.await.unwrap();
    t2.await.unwrap();
}
