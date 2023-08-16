use std::thread;

use crossbeam::channel;
use rustyline::{error::ReadlineError, DefaultEditor};
use tracing_unwrap::*;
use wigglyair::configuration;

#[tokio::main]
async fn main() {
    let _guard = configuration::setup_tracing_async("testrig".into());

    tracing::info!("doing it");
    let mut rl = DefaultEditor::new().unwrap_or_log();

    if rl.load_history("history.txt").is_err() {
        tracing::debug!("no previous history");
    }

    // create a channel to send a string
    let (tx, rx) = channel::unbounded::<String>();

    let h1 = thread::spawn(move || {
        loop {
            let readline = rl.readline(">> ");
            match readline {
                Ok(line) => {
                    if let Err(error) = rl.add_history_entry(line.as_str()) {
                        tracing::debug!(%error, "failed to add history entry");
                    };

                    if let Err(error) = tx.send(line) {
                        tracing::debug!(%error, "failed to send message");
                    };
                }
                Err(ReadlineError::Interrupted) => {
                    println!("CTRL-C");
                    break;
                }
                Err(ReadlineError::Eof) => {
                    println!("CTRL-D");
                    break;
                }
                Err(err) => {
                    println!("Error: {:?}", err);
                    break;
                }
            }
        }
        if let Err(error) = rl.save_history("history.txt") {
            tracing::debug!(%error, "failed to save history");
        };
    });

    let h2 = thread::spawn(move || {
        while let Ok(msg) = rx.recv() {
            if msg == "" {
                break;
            }
            println!("got message: {} (len={})", msg, msg.len());
        }
    });

    h1.join().unwrap();
    h2.join().unwrap();
}
