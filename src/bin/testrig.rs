use std::{
    error::Error,
    io::{self, Stdout},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use itertools::FoldWhile::*;
use itertools::Itertools;
use metaflac::Tag;
use ratatui::{prelude::*, widgets::*};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[clap(help = "Files to play. Must be flac")]
    files: Vec<String>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    let tracks: TrackList = cli
        .files
        .iter()
        .map(|f| Path::new(f))
        .filter(|p| p.exists() && p.extension().unwrap_or_default() == "flac")
        .map(|p| (p, Tag::read_from_path(p).unwrap()))
        .map(|(p, tag)| {
            let path = p.to_owned();
            let samples = tag
                .get_streaminfo()
                .map(|si| si.total_samples)
                .unwrap_or_default();
            Track { path, samples }
        })
        .collect_vec()
        .into();

    let current_sample = Arc::new(AtomicU64::new(0));

    let mut terminal = setup_terminal()?;
    start_sample_ticker(Arc::clone(&current_sample), tracks.total_samples);
    run(&mut terminal, tracks, Arc::clone(&current_sample))?;
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

fn start_sample_ticker(current_sample: Arc<AtomicU64>, total_samples: u64) -> JoinHandle<()> {
    thread::spawn(move || {
        current_sample.store(0, Ordering::SeqCst);
        loop {
            thread::sleep(Duration::from_millis(100));
            let delta = 9600;
            let previous = current_sample.fetch_add(delta, Ordering::SeqCst);
            if previous + delta >= total_samples {
                break;
            }
        }
    })
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    tracks: TrackList,
    current_sample: Arc<AtomicU64>,
) -> Result<(), Box<dyn Error>> {
    let tracks = Rc::new(tracks);
    let total_samples = tracks.total_samples;
    let mut volume = 100i16;
    Ok(loop {
        let current_sample = current_sample.load(Ordering::SeqCst);
        let ratio = current_sample as f64 / total_samples as f64;
        let current_track = tracks.find_playing(current_sample);
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

            let items = tracks
                .tracks
                .iter()
                .map(|t| {
                    let style = Style::default();
                    let style = if t == current_track {
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

#[derive(Debug, Clone, PartialEq)]
struct Track {
    path: PathBuf,
    samples: u64,
}

struct TrackList {
    tracks: Vec<Track>,
    total_samples: u64,
}

#[allow(dead_code)]
impl TrackList {
    fn new() -> Self {
        Self {
            tracks: Vec::new(),
            total_samples: 0,
        }
    }

    fn add_track(&mut self, track: Track) {
        self.total_samples += track.samples;
        self.tracks.push(track);
    }

    fn add_tracks(&mut self, tracks: Vec<Track>) {
        self.total_samples += tracks.iter().map(|t| t.samples).sum::<u64>();
        self.tracks.extend(tracks);
    }

    fn find_playing(&self, current_sample: u64) -> &Track {
        let (found, _) = self
            .tracks
            .iter()
            .enumerate()
            .fold_while((0usize, 0u64), |(i, mut total), (j, track)| {
                total += track.samples;
                if total > current_sample {
                    Done((i, total))
                } else {
                    Continue((j, total))
                }
            })
            .into_inner();
        &self.tracks[found]
    }
}

impl Default for TrackList {
    fn default() -> Self {
        Self::new()
    }
}

impl Into<TrackList> for Vec<Track> {
    fn into(self) -> TrackList {
        let mut tl = TrackList::new();
        tl.add_tracks(self);
        tl
    }
}
