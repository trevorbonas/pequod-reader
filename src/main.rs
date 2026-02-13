use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use ratatui::Terminal;
use tokio::sync::mpsc;

mod app;
mod tui;

use crate::app::{App, AppEvent};
use crate::tui::{get_rows, ui};

fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    receiver: &mut mpsc::UnboundedReceiver<AppEvent>,
) -> Result<()> {
    let tick_rate = Duration::from_millis(100);
    let mut last_tick = Instant::now();
    loop {
        terminal
            .draw(|f| ui(app, f))
            .map_err(|e| anyhow!("Failed to draw: {}", e))?;
        let rows = get_rows(app);

        // Asynchronous operations.
        if let Ok(app_event) = receiver.try_recv() {
            app.handle_app_event(app_event);
        }

        if last_tick.elapsed() >= tick_rate {
            app.on_tick();
            last_tick = Instant::now();
        }

        // Keyboard input.
        if event::poll(std::time::Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if app.handle_key(key, &rows)? {
                    return Ok(());
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    enable_raw_mode()?;
    let mut terminal = ratatui::init();
    let mut app = App::new(sender);
    let _ = run_app(&mut terminal, &mut app, &mut receiver);
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        ratatui::crossterm::event::DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}
