use std::io::stdout;
use std::time::Duration;

use crossterm::{
    event::{self, Event as CEvent, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{backend::CrosstermBackend, prelude::*, widgets::*};

use crate::{rx_loop, tx};

pub async fn chat(
    adapter: btleplug::platform::Adapter,
    topic: u8,
    ttl: u8,
    key: Option<crate::crypto::KeyBytes>,
    rate: f64,
) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;

    let (msg_tx, mut msg_rx) = tokio::sync::mpsc::unbounded_channel::<([u8; 4], String, u8)>();

    // spawn receiver
    let adapter_rx = adapter.clone();
    let key_rx = key.clone();
    tokio::spawn(async move {
        let _ = rx_loop(adapter_rx, Some(topic), true, key_rx, move |t, id, text| {
            let _ = msg_tx.send((id, text, t));
        })
        .await;
    });

    let mut input = String::new();
    let mut messages: Vec<([u8; 4], String, u8)> = Vec::new();

    loop {
        terminal.draw(|f| {
            let areas = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(20), Constraint::Min(1)].as_ref())
                .split(f.size());
            let rooms = List::new(vec![ListItem::new(format!("{:#04x}", topic))])
                .block(Block::default().title("Rooms").borders(Borders::ALL));
            f.render_widget(rooms, areas[0]);

            let inner = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(3)].as_ref())
                .split(areas[1]);
            let lines: Vec<Line> = messages
                .iter()
                .map(|(id, msg, _)| {
                    let color = Color::Indexed((id[0] % 216) + 16);
                    Line::styled(msg.clone(), Style::default().fg(color))
                })
                .collect();
            let msg_box = Paragraph::new(lines)
                .block(Block::default().borders(Borders::ALL).title("Messages"));
            f.render_widget(msg_box, inner[0]);
            let inp = Paragraph::new(input.as_str())
                .block(Block::default().borders(Borders::ALL).title("Input"));
            f.render_widget(inp, inner[1]);
        })?;

        while let Ok(m) = msg_rx.try_recv() {
            messages.push(m);
        }

        if event::poll(Duration::from_millis(50))? {
            if let CEvent::Key(kev) = event::read()? {
                match kev.code {
                    KeyCode::Char(c) => input.push(c),
                    KeyCode::Backspace => {
                        input.pop();
                    }
                    KeyCode::Enter => {
                        let m = input.clone();
                        input.clear();
                        // UI needs its own copy since we move `m` into the task
                        let ui_copy = m.clone();
                        let adapter_tx = adapter.clone();
                        let key_tx = key.clone();
                        tokio::spawn(async move {
                            let _ = tx(adapter_tx, topic, ttl, &m, 500, rate, key_tx).await;
                        });
                        messages.push(([0; 4], ui_copy, topic));
                    }
                    KeyCode::Esc => break,
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    let mut out = std::io::stdout();
    execute!(out, LeaveAlternateScreen)?;
    Ok(())
}
