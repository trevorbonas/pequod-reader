//! The terminal UI.

use chrono::Local;
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
use unicode_width::{self, UnicodeWidthChar, UnicodeWidthStr};

use crate::app::App;

pub const SPINNER_CHARS: &[char] = &['/', '-', '\\', '|'];

/// A row in the list view. A row can be either an RSS feed
/// or an entry belonging to an RSS feed.
pub enum Row {
    RssFeed(usize),         // Feed index.
    RssEntry(usize, usize), // Feed index and entry index.
}

/// View states the reader supports.
#[derive(Debug, Default, PartialEq, Eq)]
pub enum ViewState {
    #[default]
    /// A list of feeds with nested entries.
    RssFeeds,
    /// An entry, displaying entry content.
    RssEntry {
        rss_feed_index: usize,
        rss_entry_index: usize,
    },
}

/// The popup state, representing a type of popup that can
/// be displayed.
#[derive(PartialEq)]
pub enum PopupState {
    None,
    /// The popup for adding a new feed. Accepts user input.
    AddRssFeed,
    /// The popup asking the user to confirm the deletion of
    /// an RSS feed.
    ConfirmDeleteRssFeed,
    /// The popup that displays errors.
    Error,
    /// The popup that displays keybinds for navigating
    /// an RSS entry.
    RssEntryHelp,
    /// The popup that displays keybinds for navigating
    /// the list of RSS feeds.
    RssFeedHelp,
    /// The popup that indicates that syncing is happening.
    Syncing,
}

/// Draws the UI.
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
    if let PopupState::Syncing = app.popup {
        draw_syncing_popup(frame, app);
    }
    if let PopupState::Error = app.popup {
        if let Some(error_message) = app.error_message.clone() {
            draw_error_popup(frame, &error_message);
        }
    }
}

/// Draws the list of RSS feeds and their entries.
fn draw_list(frame: &mut ratatui::Frame, app: &mut App) {
    let area = frame.area();
    app.last_frame_area = area;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)])
        .split(area);

    let rows = get_rows(app);
    let visible_height = area.height as usize - 2;
    let start = if rows.len() <= visible_height {
        0
    } else if app.cursor < visible_height / 2 {
        0
    } else if app.cursor + visible_height / 2 >= rows.len() {
        rows.len() - visible_height
    } else {
        app.cursor - visible_height / 2
    };
    let visible_rows = &rows[start..start + visible_height.min(rows.len() - start)];

    let items: Vec<ListItem> = visible_rows
        .iter()
        .map(|row| match row {
            Row::RssFeed(rss_feed_index) => {
                let rss_feed = &app.rss_feeds[*rss_feed_index];
                let mut spans: Vec<Span> = Vec::new();
                let prefix = if rss_feed.expanded {
                    Span::raw("▼ ")
                } else {
                    Span::raw("▶ ")
                };
                spans.push(prefix);
                let truncated_title = truncate_str(&rss_feed.title, area.width as usize);
                spans.push(Span::raw(truncated_title));
                let num_unread_rss_entries = rss_feed
                    .rss_entries
                    .iter()
                    .filter(|a| a.read == false)
                    .count();
                let num_unread_rss_entries_formatted =
                    Span::raw(format!(" {}*", num_unread_rss_entries)).fg(Color::Rgb(255, 179, 0));
                let postfix = if num_unread_rss_entries == 0 {
                    Span::default()
                } else {
                    num_unread_rss_entries_formatted
                };
                spans.push(postfix);

                ListItem::new(Line::from(spans))
            }
            Row::RssEntry(rss_feed_index, rss_entry_index) => {
                let rss_entry = &app.rss_feeds[*rss_feed_index].rss_entries[*rss_entry_index];
                let wrapped_width = if area.width.saturating_sub(24) > 0 {
                    area.width.saturating_sub(24)
                } else {
                    area.width
                };
                let wrapped_title = textwrap::wrap(&rss_entry.title, (wrapped_width) as usize);
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
    state.select(Some(app.cursor.saturating_sub(start)));

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

/// Truncates a string to a specific width.
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
        if width + ch_width > max_width - 3 {
            break;
        }
        result.push(ch);
        width += ch_width;
    }
    result.push_str("...");
    result
}

/// Draws the contents of an RSS entry.
fn draw_rss_entry(
    frame: &mut ratatui::Frame,
    app: &mut App,
    rss_feed_index: usize,
    rss_entry_index: usize,
) {
    let size = frame.area();
    app.last_frame_area = size;
    let rss_entry = &mut app.rss_feeds[rss_feed_index].rss_entries[rss_entry_index];
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
    rss_entry.content_total_lines = wrapped_lines.len();
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

/// Retrieves all current rows.
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

/// Draws the RSS entry help popup, which displays keybinds used
/// for navigating an RSS entry.
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

/// Draws the RSS feed popup, which shows keybinds used for
/// navigating the list of RSS feeds.
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

/// Draws the popup for adding a new RSS feed.
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

/// Draws the popup that confirms whether the users wants to delete an
/// RSS feed.
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

/// Draws the popup that indicates that syncing is happening.
fn draw_syncing_popup(frame: &mut ratatui::Frame, app: &mut App) {
    let area = frame.area();
    let spinner_char = SPINNER_CHARS[app.spinner_index];
    let syncing_text = format!("Syncing {}", spinner_char);
    let paragraph = Paragraph::new(syncing_text)
        .style(Style::default().fg(Color::Rgb(255, 239, 0)))
        .centered()
        .block(Block::bordered().fg(Color::Rgb(255, 239, 0)));
    let vertical = Layout::vertical([Constraint::Length(3)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Percentage(85)]).flex(Flex::Center);
    let popup_area = area;
    let [popup_area] = vertical.areas(popup_area);
    let [popup_area] = horizontal.areas(popup_area);

    frame.render_widget(Clear, popup_area);
    frame.render_widget(paragraph, popup_area);
}

/// Draws the error popup, which an error message.
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

/// Wraps a string to a particular width.
fn wrap_str(text: &str, width: usize) -> Vec<String> {
    let options = textwrap::Options::new(width).break_words(false);
    textwrap::wrap(text, options)
        .into_iter()
        .map(|l| l.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests wrapping a string containing two words.
    #[test]
    fn test_wrap_str_two_words() {
        let test_title = "two words";
        let wrapped_str = wrap_str(test_title, 1);
        assert!(wrapped_str.first().unwrap() == "two");
        assert!(wrapped_str.get(1).unwrap() == "words");
    }

    /// Tests wrapping an empty string.
    #[test]
    fn test_wrap_str_empty() {
        let test_title = String::new();
        let wrapped_str = wrap_str(&test_title, 3);
        assert!(wrapped_str.first().unwrap() == "");
        assert!(wrapped_str.get(1).is_none());
    }

    /// Tests attempting to wrap a long, single-word string.
    #[test]
    fn test_wrap_str_long_single_world() {
        let test_title = "this_is_a_long_word_that_should_not_get_wrapped";
        let wrapped_str = wrap_str(test_title, 1);
        assert!(wrapped_str.first().unwrap() == "this_is_a_long_word_that_should_not_get_wrapped");
        assert!(wrapped_str.get(1).is_none());
    }

    /// Tests truncating a title.
    #[test]
    fn test_truncate_str_simple() {
        let test_title = "test_title";
        let truncated_title = truncate_str(&test_title, 7);
        assert!(truncated_title == "test...");
    }

    /// Tests truncating an empty string.
    #[test]
    fn test_truncate_str_empty() {
        let test_title = String::new();
        let truncated_title = truncate_str(&test_title, 7);
        assert!(truncated_title == "");
    }
}
