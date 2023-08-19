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
    let mut volume = 0i32;
    Ok(loop {
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints([Constraint::Length(4), Constraint::Length(1)].as_ref())
                .split(f.size());
            let top_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .margin(1)
                .constraints([Constraint::Length(8), Constraint::Length(1)].as_ref())
                .split(chunks[0]);

            // put volume in top left chunk
            let text = vec![
                Line::from(vec![Span::raw("volume")]),
                Line::from(vec![Span::styled(
                    format!("{:02}", volume),
                    Style::default().fg(Color::Red),
                )]),
            ];
            let paragraph = Paragraph::new(text).alignment(Alignment::Center);
            f.render_widget(paragraph, top_chunks[0]);

            let text = vec![
                Line::from(vec![
                    Span::raw("First "),
                    Span::styled("line", Style::default().add_modifier(Modifier::ITALIC)),
                    Span::raw("."),
                ]),
                Line::from(Span::styled("Second line", Style::default().fg(Color::Red))),
            ];
            let paragraph = Paragraph::new(text)
                .block(Block::default().title("Paragraph").borders(Borders::ALL))
                .style(Style::default().fg(Color::White).bg(Color::Black))
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true });
            f.render_widget(paragraph, top_chunks[1]);

            let block = Block::default().title("🎶🎶🎶").borders(Borders::ALL);
            f.render_widget(block, chunks[1]);
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

fn volume_modifier(key: KeyEvent) -> i32 {
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
