use std::{
    error::Error,
    io::{self, Stdout},
    path::Path,
    time::Duration,
};

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use itertools::Itertools;
use ratatui::{prelude::*, widgets::*};
// use rusqlite::{Connection, Result as SqlResult};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[clap(help = "Files to play. Must be flac")]
    files: Vec<String>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    let files = cli
        .files
        .iter()
        .map(|f| Path::new(f))
        .filter(|p| p.exists() && p.extension().unwrap_or_default() == "flac")
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    assert!(!files.is_empty(), "No files to play");

    let mut terminal = setup_terminal()?;
    run(&mut terminal, files)?;
    restore_terminal(&mut terminal)?;
    Ok(())
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>, Box<dyn Error>> {
    let mut stdout = io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen)?;
    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

fn restore_terminal(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> Result<(), Box<dyn Error>> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen,)?;
    Ok(terminal.show_cursor()?)
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    files: Vec<String>,
) -> Result<(), Box<dyn Error>> {
    let mut volume = 100i16;
    Ok(loop {
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints(
                    [
                        Constraint::Length(1),
                        Constraint::Min(1),
                        Constraint::Length(1),
                    ]
                    .as_ref(),
                )
                .split(f.size());

            let gauge = Gauge::default()
                .gauge_style(Style::default().fg(Color::Magenta).bg(Color::Black))
                .percent(volume as u16);
            f.render_widget(gauge, chunks[0]);

            let items = files
                .iter()
                .map(|f| ListItem::new(f.as_str()))
                .enumerate()
                .map(|(i, mut li)| {
                    if i == 3 {
                        li = li.style(Style::default().fg(Color::Green).bold());
                    }
                    li
                })
                .collect_vec();
            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL))
                .style(Style::default().fg(Color::White));
            f.render_widget(list, chunks[1]);

            let gauge = Gauge::default()
                .gauge_style(Style::default().fg(Color::Yellow).bg(Color::Black))
                .ratio(0.35);
            f.render_widget(gauge, chunks[2]);
        })?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Char('c') if is_holding_ctrl(key) => break,
                    KeyCode::Up => {
                        let n = volume_modifier(key);
                        volume = if volume + n < 100 { volume + n } else { 100 }
                    }
                    KeyCode::Down => {
                        let n = volume_modifier(key);
                        volume = if volume - n > 0 { volume - n } else { 0 };
                    }
                    _ => {}
                }
            }
        }
    })
}

fn volume_modifier(key: KeyEvent) -> i16 {
    if is_holding_shift(key) {
        10
    } else {
        1
    }
}

fn is_holding_shift(key: KeyEvent) -> bool {
    key.modifiers.contains(event::KeyModifiers::SHIFT)
}

fn is_holding_ctrl(key: KeyEvent) -> bool {
    key.modifiers.contains(event::KeyModifiers::CONTROL)
}
