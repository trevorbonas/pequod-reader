//! A single-page TUI RSS reader with a small footprint.

use std::path::PathBuf;
use std::str::FromStr;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use clap::Parser;
use crossterm::event::{self, Event};
use crossterm::terminal::enable_raw_mode;
use ratatui::Terminal;
use tokio::sync::mpsc;

mod app;
mod local_storage;
mod tui;

use crate::app::{App, AppEvent};
use crate::tui::{get_rows, ui};

/// Runs the application.
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

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    db_path: Option<String>,

    #[arg(short, long)]
    max_ttl_days: Option<usize>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli: Cli = Cli::parse();
    let db_path = match cli.db_path {
        Some(db_path) => PathBuf::from_str(db_path.as_str()).ok(),
        None => None,
    };
    let max_ttl_days = match cli.max_ttl_days {
        Some(max_ttl_days) => Some(chrono::Duration::days(max_ttl_days as i64)),
        None => None,
    };

    let (sender, mut receiver) = mpsc::unbounded_channel();
    enable_raw_mode()?;
    let mut terminal = ratatui::init();
    let mut app = App::new(sender, db_path, max_ttl_days)?;
    let _ = run_app(&mut terminal, &mut app, &mut receiver);
    ratatui::restore();

    Ok(())
}
