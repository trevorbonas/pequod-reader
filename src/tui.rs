use chrono::Local;
use crossterm::execute;
use crossterm::terminal::{LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use ratatui::Terminal;
use ratatui::layout::{Constraint, Direction, Flex, Layout, Margin, Position};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{
    Borders, Clear, List, ListItem, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
};
use ratatui::{
    Frame,
    style::Stylize,
    text::Line,
    widgets::{Block, Paragraph},
};
use std::io::{self, Stdout, stdout};
use unicode_width::{self, UnicodeWidthChar, UnicodeWidthStr};

use ratatui::{backend::CrosstermBackend, crossterm::terminal::EnterAlternateScreen};

use crate::app::App;

pub enum Row {
    RssFeed(usize),         // Feed index.
    RssEntry(usize, usize), // Feed index and entry index.
}

#[derive(Debug, Default, PartialEq, Eq)]
pub enum ViewState {
    #[default]
    // A list of feeds with nested entries.
    RssFeeds,
    // An entry title with a date and author(s).
    RssEntry {
        rss_feed_index: usize,
        rss_entry_index: usize,
    },
}

pub enum PopupState {
    None,
    // Popup for adding a new feed. Takes user input.
    AddRssFeed,
    ConfirmDeleteRssFeed,
    // Popup that displays errors.
    Error,
    // Help popup displaying keybinds and helpful information.
    RssEntryHelp,
    RssFeedHelp,
}

/// A type alias for the terminal type used in this application
pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Initialize the terminal
pub fn init() -> io::Result<Tui> {
    execute!(stdout(), EnterAlternateScreen)?;
    enable_raw_mode()?;
    set_panic_hook();
    Terminal::new(CrosstermBackend::new(stdout()))
}

fn set_panic_hook() {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = restore(); // ignore any errors as we are already failing
        hook(panic_info);
    }));
}

/// Restore the terminal to its original state
pub fn restore() -> io::Result<()> {
    execute!(stdout(), LeaveAlternateScreen)?;
    disable_raw_mode()?;
    Ok(())
}

pub fn ui(app: &mut App, frame: &mut Frame) {
    match app.view_state {
        ViewState::RssFeeds => draw_list(frame, app),
        ViewState::RssEntry {
            rss_feed_index,
            rss_entry_index,
        } => draw_rss_entry(frame, app, rss_feed_index, rss_entry_index),
    }

    if let PopupState::RssEntryHelp = app.popup {
        draw_rss_entry_help_popup(frame);
    }
    if let PopupState::RssFeedHelp = app.popup {
        draw_rss_feed_help_popup(frame);
    }
    if let PopupState::AddRssFeed = app.popup {
        draw_add_rss_feed_popup(frame, app);
    }
    if let PopupState::ConfirmDeleteRssFeed = app.popup {
        draw_confirm_delete_rss_feed_popup(frame, app);
    }
    if let Some(error_message) = app.error_message.clone() {
        app.popup = PopupState::Error;
        draw_error_popup(frame, &error_message);
    }
}

fn draw_list(frame: &mut ratatui::Frame, app: &App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)])
        .split(area);

    let rows = get_rows(app);

    let items: Vec<ListItem> = rows
        .iter()
        .map(|row| match row {
            Row::RssFeed(rss_feed_index) => {
                let feed = &app.rss_feeds[*rss_feed_index];
                let num_unread_rss_entries =
                    feed.rss_entries.iter().filter(|a| a.read == false).count();
                let num_unread_rss_entries_formatted = format!(" {}*", num_unread_rss_entries);
                let postfix = if num_unread_rss_entries == 0 {
                    ""
                } else {
                    &num_unread_rss_entries_formatted
                };
                let prefix = if feed.expanded { "▼ " } else { "▶ " };
                ListItem::new(format!("{}{}{}", prefix, feed.title, postfix))
            }
            Row::RssEntry(rss_feed_index, rss_entry_index) => {
                let rss_entry = &app.rss_feeds[*rss_feed_index].rss_entries[*rss_entry_index];
                let wrapped_title = textwrap::wrap(&rss_entry.title, (area.width - 24) as usize);
                let date = rss_entry
                    .published
                    .unwrap_or_default()
                    .with_timezone(&Local)
                    .format("%Y-%m-%d %I:%M%P")
                    .to_string();

                let mut lines: Vec<Line> = Vec::new();

                for (i, wrapped_line) in wrapped_title.iter().enumerate() {
                    let mut spans: Vec<Span> = Vec::new();
                    spans.push(Span::raw("    "));
                    spans.push(Span::raw(wrapped_line.to_string()));
                    if i == wrapped_title.len() - 1 {
                        if !rss_entry.read {
                            spans.push(Span::styled(
                                "*",
                                Style::default().fg(Color::Rgb(255, 179, 0)),
                            ));
                        }
                        spans.push(Span::styled(format!(" {}", date), Style::default().dim()));
                    }
                    lines.push(Line::from(spans));
                }

                ListItem::from(lines)
            }
        })
        .collect();

    let instructions = Line::from(vec![
        " ↓".into(),
        "<j> ".blue().bold().into(),
        "↑".into(),
        "<k> ".blue().bold().into(),
        "Select".into(),
        "<Enter> ".blue().bold().into(),
        "Add".into(),
        "<a> ".blue().bold().into(),
        "Delete".into(),
        "<d> ".blue().bold().into(),
        "Sync".into(),
        "<s> ".blue().bold().into(),
        "Quit".into(),
        "<q> ".blue().bold().into(),
    ]);

    let list = List::new(items)
        .block(
            Block::default()
                .title("Feeds".bold())
                .borders(Borders::ALL)
                .title_bottom(instructions.centered()),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ratatui::widgets::ListState::default();
    state.select(Some(app.cursor));

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
    frame.render_stateful_widget(
        scrollbar,
        chunks[0].inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut ScrollbarState::default(),
    );
    frame.render_stateful_widget(list, area, &mut state);
}

fn truncate_str(str_to_truncate: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(str_to_truncate) <= max_width {
        return str_to_truncate.to_string();
    }

    if max_width <= 3 {
        return ".".repeat(max_width);
    }
    let mut result = String::new();
    let mut width = 0;
    for ch in str_to_truncate.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width >= max_width - 3 {
            break;
        }
        result.push(ch);
        width += ch_width;
    }
    result.push_str("...");
    result
}

fn draw_rss_entry(
    frame: &mut ratatui::Frame,
    app: &mut App,
    rss_feed_index: usize,
    rss_entry_index: usize,
) {
    let size = frame.area();
    app.last_rss_entry_area = size;
    let rss_entry = &app.rss_feeds[rss_feed_index].rss_entries[rss_entry_index];
    let instructions = Line::from(vec![
        " ↓".into(),
        "<j> ".blue().bold().into(),
        "↑".into(),
        "<k> ".blue().bold().into(),
        "Fetch".into(),
        "<f> ".blue().bold().into(),
        "Open".into(),
        "<o> ".blue().bold().into(),
        "Help".into(),
        "<h> ".blue().bold().into(),
        "Back".into(),
        "<q> ".blue().bold().into(),
    ]);
    let wrapped_lines = wrap_str(&rss_entry.content, (frame.area().width - 2) as usize);
    let visible_lines = wrapped_lines
        .iter()
        .skip(app.rss_entry_scroll as usize)
        .take(size.height as usize);
    let text = visible_lines
        .map(|l| Line::from(l.clone()))
        .collect::<Vec<_>>();
    let truncated_title = truncate_str(&rss_entry.title, (frame.area().width - 2) as usize);
    let paragraph = Paragraph::new(text).block(
        Block::default()
            .title(truncated_title.clone().bold())
            .title_bottom(instructions.centered())
            .borders(Borders::ALL),
    );
    frame.render_widget(paragraph, size);
}

pub fn get_rows(app: &App) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();

    for (rss_feed_index, rss_feed) in app.rss_feeds.iter().enumerate() {
        rows.push(Row::RssFeed(rss_feed_index));
        if rss_feed.expanded {
            for (rss_entry_index, _) in rss_feed.rss_entries.iter().enumerate() {
                rows.push(Row::RssEntry(rss_feed_index, rss_entry_index));
            }
        }
    }
    rows
}

fn draw_rss_entry_help_popup(frame: &mut ratatui::Frame) {
    let area = frame.area();
    let instructions = Line::from(vec![" Back".into(), "<q> ".blue().bold().into()]);
    let paragraph = Paragraph::new(String::default())
        .style(Style::default())
        .block(
            Block::bordered()
                .title("Entry commands")
                .title_bottom(instructions.centered()),
        );
    let vertical = Layout::vertical([Constraint::Length(3)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Percentage(85)]).flex(Flex::Center);
    let popup_area = area;
    let [popup_area] = vertical.areas(popup_area);
    let [popup_area] = horizontal.areas(popup_area);

    frame.render_widget(Clear, popup_area);
    frame.render_widget(paragraph, popup_area);
}

fn draw_rss_feed_help_popup(frame: &mut ratatui::Frame) {
    let area = frame.area();
    let instructions = Line::from(vec![" Back".into(), "<q> ".blue().bold().into()]);
    let paragraph = Paragraph::new(String::default())
        .style(Style::default())
        .block(
            Block::bordered()
                .title("Feed commands")
                .title_bottom(instructions.centered()),
        );
    let vertical = Layout::vertical([Constraint::Length(3)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Percentage(85)]).flex(Flex::Center);
    let popup_area = area;
    let [popup_area] = vertical.areas(popup_area);
    let [popup_area] = horizontal.areas(popup_area);

    frame.render_widget(Clear, popup_area);
    frame.render_widget(paragraph, popup_area);
}

fn draw_add_rss_feed_popup(frame: &mut ratatui::Frame, app: &mut App) {
    let area = frame.area();
    let instructions = Line::from(vec![
        " Submit".into(),
        "<Enter> ".blue().bold().into(),
        "Back".into(),
        "<q> ".blue().bold().into(),
    ]);
    let input_paragraph = Paragraph::new(app.input.as_str())
        .style(Style::default().fg(Color::Rgb(255, 161, 0)))
        .block(
            Block::bordered()
                .title("Add feed")
                .title_bottom(instructions.centered()),
        );
    let vertical = Layout::vertical([Constraint::Length(3)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Percentage(85)]).flex(Flex::Center);
    let popup_area = area;
    let [popup_area] = vertical.areas(popup_area);
    let [popup_area] = horizontal.areas(popup_area);
    let [input_area] = vertical.areas(popup_area);

    #[allow(clippy::cast_possible_truncation)]
    frame.set_cursor_position(Position::new(
        input_area.x + app.character_index as u16 + 1,
        input_area.y + 1,
    ));

    frame.render_widget(Clear, popup_area);
    frame.render_widget(input_paragraph, popup_area);
}

fn draw_confirm_delete_rss_feed_popup(frame: &mut ratatui::Frame, app: &mut App) {
    let rows = get_rows(app);
    let row = &rows[app.cursor];
    let rss_feed_name = match row {
        Row::RssFeed(rss_feed_index) | Row::RssEntry(rss_feed_index, _) => {
            app.rss_feeds[*rss_feed_index].title.as_str()
        }
    };
    let area = frame.area();
    let instructions = Line::from(vec![
        " Yes".into(),
        "<y> ".blue().bold().into(),
        "No".into(),
        "<n> ".blue().bold().into(),
        "Cancel".into(),
        "<q> ".blue().bold().into(),
    ]);

    let horizontal = Layout::horizontal([Constraint::Percentage(85)]).flex(Flex::Center);
    let [popup_area] = horizontal.areas(area);
    let text_width = popup_area.width;
    let text = format!(
        "Are you sure that you want to delete feed \"{}\"",
        rss_feed_name
    );
    let wrapped_text = wrap_str(&text, text_width as usize);
    let height = wrapped_text.len() + 2;

    let paragraph = Paragraph::new(text)
        .style(Style::default().fg(Color::Rgb(255, 0, 0)))
        .block(
            Block::bordered()
                .fg(Color::Rgb(255, 0, 0))
                .title("Delete feed")
                .title_bottom(instructions.centered()),
        )
        .wrap(Wrap { trim: true });

    let vertical = Layout::vertical([Constraint::Length(height as u16)]).flex(Flex::Center);
    let [popup_area] = vertical.areas(popup_area);

    frame.render_widget(Clear, popup_area);
    frame.render_widget(paragraph, popup_area);
}

fn draw_error_popup(frame: &mut ratatui::Frame, error_message: &str) {
    let area = frame.area();
    let instructions = Line::from(vec![" Ok".into(), "<Enter> ".blue().bold().into()]);
    let paragraph = Paragraph::new(format!("Error: {}", error_message))
        .style(Style::default().fg(Color::Rgb(255, 0, 0)))
        .block(
            Block::bordered()
                .fg(Color::Rgb(255, 0, 0))
                .title("Error")
                .title_bottom(instructions.centered()),
        );
    let vertical = Layout::vertical([Constraint::Length(3)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Percentage(85)]).flex(Flex::Center);
    let popup_area = area;
    let [popup_area] = vertical.areas(popup_area);
    let [popup_area] = horizontal.areas(popup_area);

    frame.render_widget(Clear, popup_area);
    frame.render_widget(paragraph, popup_area);
}

fn wrap_str(text: &str, width: usize) -> Vec<String> {
    let options = textwrap::Options::new(width).break_words(false);
    textwrap::wrap(text, options)
        .into_iter()
        .map(|l| l.to_string())
        .collect()
}
