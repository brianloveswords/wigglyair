use std::{
    error::Error,
    io::{self, Stdout},
    time::Duration,
};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{prelude::*, widgets::*};

fn main() -> Result<(), Box<dyn Error>> {
    let mut terminal = setup_terminal()?;
    run(&mut terminal)?;
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

fn run(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<(), Box<dyn Error>> {
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

            let block = Block::default().title("🎶🎶🎶").borders(Borders::ALL);
            let items = [
                ListItem::new("Item 1"),
                ListItem::new("Item 2").set_style(Style::default().fg(Color::Green).bold()),
                ListItem::new("Item 3"),
            ];
            let list = List::new(items)
                .block(block)
                .style(Style::default().fg(Color::White));
            f.render_widget(list, chunks[1]);

            let gauge = Gauge::default()
                .gauge_style(Style::default().fg(Color::Yellow).bg(Color::Black))
                .percent(volume as u16);
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
