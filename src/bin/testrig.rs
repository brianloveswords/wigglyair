use std::{io::Write, sync::Arc, thread, time::Duration};

use clap::Parser;
use tracing_unwrap::*;
use wigglyair::{configuration, types::Volume};

#[derive(Debug, Parser)]
struct Cli {
    #[clap(short, long, default_value = "100")]
    volume: u8,
}

#[tokio::main]
async fn main() {
    let _guard = configuration::setup_tracing_async("testrig".into());

    let cli = Cli::parse();
    let volume = {
        let v = Volume::try_from(cli.volume).expect_or_log("Could not parse volume");
        Arc::new(v)
    };

    let vol1 = volume.clone();
    let h1 = thread::spawn(move || loop {
        write!(std::io::stderr(), "> ").unwrap();

        let mut msg = String::new();
        if let Err(error) = std::io::stdin().read_line(&mut msg) {
            tracing::error!("error reading line: {}", error);
            break;
        }
        if msg == "" {
            break;
        }
        match vol1.set_from_string(msg) {
            Ok(()) => tracing::debug!(volume = vol1.get(), "set volume"),
            Err(error) => tracing::error!("error setting volume: {:?}", error),
        }
    });

    let h2 = thread::spawn(move || loop {
        let vol = volume.get();
        tracing::debug!(volume = vol, "reading volume");
        thread::sleep(Duration::from_secs(1));
    });

    h1.join().unwrap();
    h2.join().unwrap();
}
