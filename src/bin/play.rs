use std::{
    error::Error,
    io::{self, Stdout},
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{prelude::*, widgets::*};
use wigglyair::{
    configuration,
    types::{AudioParams, PlayState, Player, TrackList},
};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[clap(long, help = "Start paused", default_value_t = false)]
    paused: bool,

    #[clap(help = "Files to play. Must be flac")]
    files: Vec<String>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let _guard = configuration::setup_tracing_async("play".into());

    let cli = Cli::parse();
    let tracks: TrackList = TrackList::unsafe_from_files(cli.files);
    let params: AudioParams = tracks.audio_params();
    let playing = !cli.paused;

    tracing::info!("Playing {:?}", tracks);
    tracing::info!("Audio params {:?}", params);

    let mut terminal = setup_terminal()?;
    let state = PlayState::with_state(playing);
    let player = Player::with_state(tracks, state);
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
    let tracks = Arc::clone(&player.track_list);
    let current_sample = Arc::clone(&player.current_sample);
    let volume = Arc::clone(&player.volume);
    let total_samples = tracks.total_samples;
    let current_track = Arc::clone(&player.current_track);
    let play_state = Arc::clone(&player.state);
    let sample_rate = player.audio_params.sample_rate;

    // safe initial value: there are fewer than 18 quintillion
    // tracks in the known world
    let mut last_track = usize::MAX;

    player.start();

    Ok(loop {
        let current_sample = current_sample.load(Ordering::SeqCst);
        let mut ratio = current_sample as f64 / total_samples as f64;
        let is_paused = play_state.is_paused();
        let current_track = current_track.load(Ordering::SeqCst);
        let track = &tracks.tracks[current_track];

        if current_track != last_track {
            tracing::info!(?track, "Playing next track");
            last_track = current_track;
        }

        if ratio > 1.0 {
            tracing::error!(
                ratio,
                current_track,
                current_sample,
                "current_sample / total_samples ratio > 1.0; clamping"
            );
            ratio = ratio.clamp(0.0, 1.0);
        }

        terminal.draw(|f| {
            //
            // main layout chunks
            //

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

            //
            // volume bar
            //

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

            //
            // track list UI
            //
            let items = track_list_to_list_items(&tracks, current_track, is_paused);

            let color = if is_paused { Color::Red } else { Color::White };
            let list = List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(color)),
                )
                .style(Style::default().fg(Color::White));
            f.render_widget(list, chunks[1]);

            //
            // progress bar
            //

            let color = if is_paused { Color::Red } else { Color::Yellow };
            let gauge = Gauge::default()
                .gauge_style(Style::default().fg(color).bg(Color::Black))
                .ratio(ratio)
                .label(format!(
                    "{} / {}",
                    samples_to_duration_string(sample_rate, current_sample),
                    samples_to_duration_string(sample_rate, total_samples),
                ));
            f.render_widget(gauge, chunks[2]);
        })?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('c') if is_holding_ctrl(key) => {
                        tracing::info!(reason = "keypress", "Quitting: `Ctrl-C` pressed");
                        break;
                    }
                    KeyCode::Char('q') => {
                        tracing::info!(reason = "keypress", "Quitting: `q` pressed");
                        break;
                    }
                    KeyCode::Char('p') => {
                        let was_playing = play_state.toggle();
                        if was_playing {
                            tracing::info!(?track, "Pausing");
                        } else {
                            tracing::info!(?track, "Playing");
                        }
                    }
                    KeyCode::Up => {
                        let n = volume_modifier(key);
                        let from = volume.up(n);
                        let to = from + n;
                        tracing::debug!(from, to, "Volume up");
                    }
                    KeyCode::Down => {
                        let n = volume_modifier(key);
                        let from = volume.down(n);
                        let to = from - n;
                        tracing::debug!(from, to, "Volume down");
                    }
                    other => {
                        tracing::debug!(?other, "Unhandled key event");
                    }
                }
            }
        }
    })
}

fn track_list_to_list_items(
    tracks: &TrackList,
    current_track: usize,
    is_paused: bool,
) -> Vec<ListItem> {
    let tracks = &tracks.tracks;
    let mut items = Vec::with_capacity(tracks.len());
    let mut previous_album = ""; // safe initial value because album names are non-empty
    for (i, t) in tracks.iter().enumerate() {
        // print the album header when the album changes
        // if it's not the first album, toss a linebreak above as well
        if t.album != previous_album {
            if previous_album != "" {
                items.push(ListItem::new(""));
            }
            let style = Style::default().fg(Color::DarkGray).bold().underlined();
            let label = t.display_album_header();
            let item = ListItem::new(label).style(style);
            items.push(item);
            previous_album = &t.album;
        }

        let style = Style::default();
        let style = if i == current_track {
            let color = if is_paused { Color::Red } else { Color::Green };
            style.fg(color).bold()
        } else {
            style.fg(Color::White)
        };
        let label = t.display_track();
        let item = ListItem::new(label).style(style);
        items.push(item);
    }
    items
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

fn samples_to_milliseconds(sample_rate: usize, samples: u64) -> Duration {
    let seconds = samples as f64 / sample_rate as f64;
    Duration::from_secs_f64(seconds)
}

fn duration_to_human_readable(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    format!("{:02}:{:02}", minutes, seconds)
}

fn samples_to_duration_string(sample_rate: usize, samples: u64) -> String {
    let duration = samples_to_milliseconds(sample_rate, samples);
    duration_to_human_readable(duration)
}
