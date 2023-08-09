use std::fs;

use clap::Parser;
use serde::{Deserialize, Serialize};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[clap(help = "Logfile to parse")]
    logfile: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct LogEntry {
    time: String,
    msg: String,
}

impl LogEntry {
    fn direction(&self) -> Direction {
        if self.msg.contains("- START]") {
            Direction::Enter
        } else if self.msg.contains("- END]") {
            Direction::Exit
        } else {
            Direction::None
        }
    }
    fn log_type(&self) -> String {
        self.msg[1..] // drop the leading [
            .split(" - ")
            .next()
            .expect("Failed to get log type")
            .to_string()
    }

    fn timestamp(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(&self.time)
            .expect("Failed to parse timestamp")
            .with_timezone(&chrono::Utc)
    }
}

#[derive(Debug)]
enum Direction {
    Enter,
    Exit,
    None,
}

fn main() {
    let cli = Cli::parse();

    let logfile = fs::read_to_string(cli.logfile).expect("Failed to read logfile");
    let _entries = logfile
        .lines()
        .map(|line| serde_json::from_str::<LogEntry>(line).expect("Failed to parse log entry"))
        .map(|entry| (entry.log_type(), entry.timestamp(), entry.direction()))
        .for_each(|entry| println!("{:?}", entry));
}
