use anyhow::{Result, anyhow};
use crossterm::event::{self, Event, KeyCode, KeyEvent};
use crossterm::execute;
use crossterm::terminal::{LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use html2text::from_read;
use ratatui::Terminal;
use tokio::sync::mpsc;

mod app;
mod tui;

use crate::app::{App, AppEvent, RssFeed};
use crate::tui::{PopupState, Row, ViewState, get_rows, ui};

fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    receiver: &mut mpsc::UnboundedReceiver<AppEvent>,
) -> Result<()> {
    loop {
        terminal
            .draw(|f| ui(app, f))
            .map_err(|e| anyhow!("Failed to draw: {}", e))?;
        let rows = get_rows(app);

        if let Ok(app_event) = receiver.try_recv() {
            handle_app_event(app, app_event);
        }

        if event::poll(std::time::Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if handle_key(app, key, &rows)? {
                    return Ok(());
                }
            }
        }
    }
}

fn handle_app_event(app: &mut App, app_event: AppEvent) {
    match app_event {
        AppEvent::ScrapedEntry {
            rss_feed_index,
            rss_entry_index,
            result,
        } => match result {
            Ok(content) => {
                app.rss_feeds[rss_feed_index].rss_entries[rss_entry_index].content = content;
            }
            Err(err) => {
                app.error_message = Some(err);
            }
        },
        AppEvent::FeedFetched(Ok(feed)) => {
            app.error_message = None;
            let new_rss_feed = RssFeed::from(feed);
            app.rss_feeds.push(new_rss_feed);
        }
        AppEvent::FeedFetched(Err(err)) => {
            app.error_message = Some(err);
        }
    }
}

fn handle_key(app: &mut App, key: KeyEvent, rows: &[Row]) -> Result<bool> {
    match app.popup {
        PopupState::AddRssFeed => handle_add_rss_feed_popup(app, key),
        PopupState::ConfirmDeleteRssFeed => handle_delete_rss_feed_popup(app, key, rows),
        PopupState::Error => handle_error_popup(app, key),
        PopupState::RssEntryHelp => handle_rss_entry_help_popup(app, key),
        PopupState::RssFeedHelp => handle_rss_feed_help_popup(app, key),
        PopupState::None => handle_default(app, key, rows),
    }
}

fn handle_add_rss_feed_popup(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.popup = PopupState::None,
        KeyCode::Enter => {
            app.add_rss_feed();
            app.popup = PopupState::None;
        }
        KeyCode::Char(c) => app.enter_char(c),
        KeyCode::Backspace => app.delete_char(),
        KeyCode::Left => app.move_cursor_left(),
        KeyCode::Right => app.move_cursor_right(),
        _ => {}
    }
    Ok(false)
}

fn handle_delete_rss_feed_popup(app: &mut App, key: KeyEvent, rows: &[Row]) -> Result<bool> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('n') => app.popup = PopupState::None,
        KeyCode::Char('y') => {
            let row = &rows[app.cursor];
            match row {
                Row::RssFeed(rss_feed_index) => {
                    app.delete_rss_feed(*rss_feed_index);
                    if *rss_feed_index > 0 {
                        app.cursor = rss_feed_index - 1;
                    } else {
                        app.cursor = 0;
                    }
                }
                Row::RssEntry(rss_feed_index, rss_entry_index) => {
                    app.delete_rss_feed(*rss_feed_index);
                    if *rss_feed_index > 0 {
                        app.cursor = app.cursor - rss_entry_index - 2;
                    } else {
                        app.cursor = 0;
                    }
                }
            }
            app.popup = PopupState::None;
        }
        _ => {}
    }
    Ok(false)
}

fn handle_error_popup(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
            app.error_message = None;
            app.popup = PopupState::None;
        }
        _ => {}
    }
    Ok(false)
}

fn handle_rss_feed_help_popup(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.popup = PopupState::None,
        _ => {}
    }
    Ok(false)
}

fn handle_rss_entry_help_popup(app: &mut App, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.popup = PopupState::None,
        _ => {}
    }
    Ok(false)
}

fn handle_default(app: &mut App, key: KeyEvent, rows: &[Row]) -> Result<bool> {
    match app.view_state {
        ViewState::RssFeeds => handle_rss_feeds_view(app, key, rows),
        ViewState::RssEntry {
            rss_feed_index,
            rss_entry_index,
        } => handle_rss_entry_view(app, key, rss_feed_index, rss_entry_index),
    }
}

fn handle_rss_feeds_view(app: &mut App, key: KeyEvent, rows: &[Row]) -> Result<bool> {
    match key.code {
        KeyCode::Char('H') => {
            // TODO: Update to visible top.
            app.cursor = 0;
        }
        KeyCode::Char('M') => {
            // TODO: Update to visible middle.
            app.cursor = rows.len() / 2 as usize;
        }
        KeyCode::Char('L') => {
            // TODO: Update to visible bottom.
            app.cursor = rows.len() - 1;
        }
        KeyCode::Char('G') => {
            app.cursor = rows.len() - 1;
        }
        KeyCode::Esc | KeyCode::Char('q') => return Ok(true),
        KeyCode::Down | KeyCode::Char('j') => {
            if app.cursor + 1 < rows.len() {
                app.cursor += 1;
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.cursor > 0 {
                app.cursor -= 1;
            }
        }
        KeyCode::Char('a') => {
            app.popup = PopupState::AddRssFeed;
        }
        KeyCode::Char('h') => {
            app.popup = PopupState::RssFeedHelp;
        }
        KeyCode::Char('c') => match rows[app.cursor] {
            Row::RssFeed(rss_feed_index) => {
                app.rss_feeds[rss_feed_index].expanded = false;
            }
            Row::RssEntry(rss_feed_index, rss_entry_index) => {
                app.rss_feeds[rss_feed_index].expanded = false;
                app.cursor = app.cursor - rss_entry_index - 1;
            }
        },
        KeyCode::Enter => {
            if rows.len() > 0 {
                match rows[app.cursor] {
                    Row::RssFeed(rss_feed_index) => {
                        app.rss_feeds[rss_feed_index].expanded =
                            !app.rss_feeds[rss_feed_index].expanded;
                    }
                    Row::RssEntry(rss_feed_index, rss_entry_index) => {
                        app.rss_entry_scroll = 0;
                        app.rss_feeds[rss_feed_index].rss_entries[rss_entry_index].read = true;
                        app.view_state = ViewState::RssEntry {
                            rss_feed_index,
                            rss_entry_index,
                        };
                    }
                }
            }
        }
        KeyCode::Char('d') => match rows[app.cursor] {
            Row::RssFeed(_) => {
                app.popup = PopupState::ConfirmDeleteRssFeed;
            }
            Row::RssEntry(_, _) => {
                app.popup = PopupState::ConfirmDeleteRssFeed;
            }
        },
        _ => {}
    }
    Ok(false)
}

fn handle_rss_entry_view(
    app: &mut App,
    key: KeyEvent,
    rss_feed_index: usize,
    rss_entry_index: usize,
) -> Result<bool> {
    match key.code {
        KeyCode::Char('o') => {
            open::that(
                app.rss_feeds[rss_feed_index].rss_entries[rss_entry_index]
                    .link
                    .clone(),
            )?;
        }
        KeyCode::Char('f') => {
            let link = app.rss_feeds[rss_feed_index].rss_entries[rss_entry_index]
                .link
                .clone();
            let sender = app.sender.clone();

            let html_width = app.last_rss_entry_area.width;
            tokio::spawn(async move {
                let result = async {
                    let html = reqwest::get(&link)
                        .await
                        .map_err(|e| format!("Failed to load full content: {}", e.to_string()))?
                        .text()
                        .await
                        .map_err(|e| format!("Failed to load full content: {}", e.to_string()))?;
                    let parsed_html = from_read(html.as_bytes(), html_width as usize)
                        .expect("Failed to parse HTML");
                    Ok(parsed_html)
                }
                .await;

                let _ = sender.send(AppEvent::ScrapedEntry {
                    rss_feed_index,
                    rss_entry_index,
                    result,
                });
            });
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            app.view_state = ViewState::RssFeeds;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.rss_entry_scroll > 0 {
                app.rss_entry_scroll -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let area = app.last_rss_entry_area;
            let max_rss_entry_scroll =
                app.get_max_rss_entry_scroll(rss_feed_index, rss_entry_index, area.height);
            if app.rss_entry_scroll < max_rss_entry_scroll {
                app.rss_entry_scroll += 1;
            }
        }
        KeyCode::Char('h') => {
            app.popup = PopupState::RssEntryHelp;
        }
        KeyCode::End | KeyCode::Char('G') => {
            let area = app.last_rss_entry_area;
            let max_scroll =
                app.get_max_rss_entry_scroll(rss_feed_index, rss_entry_index, area.height);
            app.rss_entry_scroll = max_scroll;
        }
        _ => {}
    }
    Ok(false)
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
