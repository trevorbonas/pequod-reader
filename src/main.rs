use std::cmp::PartialEq;
use std::env::args;
use std::error::Error;
use std::fs::{File, Metadata};
use std::io::{BufRead, BufReader};
use std::path::Path;

use chrono::{DateTime, Local};
use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Flex, Layout, Margin};
use ratatui::prelude::{Color, Modifier, Style, Text};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap};

mod tui;

struct BookData {
    title: String,
    authors: Vec<String>,
    subjects: Vec<String>,
    languages: Vec<String>,
    translators: Vec<String>,
    summaries: <String>,
    current_page: u64,
    bookmarks: Vec<u64>
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = args().collect();

    let mut tui = tui::Tui::new()?
        .tick_rate(4.0)
        .frame_rate(30.0);
    tui.enter()?;

    loop {
        tui.draw(|f| {
            ui(f, &mut book_data);
        })?;

        if let Some(evt) = tui.next().await {
            let some_action = map_event(evt);
            book_data.action = some_action;

            if is_quit_action(&mut file_data) {
                break;
            }
        };
    }
    tui.exit()?;

    Ok(())
}

fn ui(frame: &mut Frame, file_data: &mut FileData) {
    let area = frame.size;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(area);

    let style_blue_bold = Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD);

    let main_content_block = Block::new()
        .borders(Borders::all())
        .padding(Padding::new(1, 1, 1, 1))
        .title(file_data.path.clone())
        .title_style(style_blue_bold);
}
