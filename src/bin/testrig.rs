use std::{
    error::Error,
    io::{self, Stdout},
    sync::atomic::Ordering,
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
use wigglyair::{
    configuration,
    types::{AudioParams, Player, TrackList},
};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[clap(help = "Files to play. Must be flac")]
    files: Vec<String>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let _guard = configuration::setup_tracing_async("testrig".into());

    let cli = Cli::parse();
    let tracks: TrackList = TrackList::from_files(cli.files);
    let params: AudioParams = tracks.audio_params();

    tracing::info!("Audio params {:?}", params);
    tracing::info!("Playing {:?}", tracks);

    let mut terminal = setup_terminal()?;
    let player = Player::new(tracks);
    run_tui(&mut terminal, player)?;
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

fn run_tui(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    player: Player,
) -> Result<(), Box<dyn Error>> {
    player.start();

    let tracks = player.track_list;
    let current_sample = player.current_sample;
    let volume = player.volume;
    let total_samples = tracks.total_samples;
    let current_track = player.current_track;
    let play_state = player.state;

    Ok(loop {
        let current_sample = current_sample.load(Ordering::SeqCst);
        let ratio = (current_sample as f64 / total_samples as f64).clamp(0.0, 1.0);
        let is_paused = play_state.is_paused();
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

            // volume
            let mut style = Style::default().bg(Color::Black).fg(Color::Magenta);
            if is_paused {
                style = style.fg(Color::Red);
            }
            let value = volume.get() as u16;
            let mut gauge = Gauge::default().gauge_style(style).percent(value);
            if is_paused {
                gauge = gauge.label("[paused]");
            }
            f.render_widget(gauge, chunks[0]);

            // track list
            let items = tracks
                .tracks
                .iter()
                .map(|t| {
                    let style = Style::default();
                    let style = if t == current_track.as_ref() {
                        style.fg(Color::Green).bold()
                    } else {
                        style.fg(Color::White)
                    };
                    ListItem::new(t.path.file_name().unwrap().to_string_lossy()).style(style)
                })
                .collect_vec();
            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL))
                .style(Style::default().fg(Color::White));
            f.render_widget(list, chunks[1]);

            // sample progress
            let gauge = Gauge::default()
                .gauge_style(Style::default().fg(Color::Yellow).bg(Color::Black))
                .ratio(ratio)
                .label(format!("{} / {}", current_sample, total_samples));
            f.render_widget(gauge, chunks[2]);
        })?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Char('p') => {
                        play_state.toggle();
                    }
                    KeyCode::Char('c') if is_holding_ctrl(key) => break,
                    KeyCode::Up => {
                        let n = volume_modifier(key);
                        volume.up(n);
                    }
                    KeyCode::Down => {
                        let n = volume_modifier(key);
                        volume.down(n);
                    }
                    _ => {}
                }
            }
        }
    })
}

fn volume_modifier(key: KeyEvent) -> u8 {
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
