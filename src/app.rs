use anyhow::Result;
use chrono::{DateTime, Utc};
use html2text::from_read;
use ratatui::layout::Rect;
use ratatui::widgets::ScrollbarState;
use std::char;
use std::cmp::{PartialEq, Reverse};
use tokio::sync::mpsc;

use crate::tui::{PopupState, ViewState};

pub struct RssFeed {
    pub id: String,
    pub title: String,
    pub link: String,
    pub rss_entries: Vec<RssEntry>,
    pub expanded: bool,
}

impl From<feed_rs::model::Feed> for RssFeed {
    fn from(feed: feed_rs::model::Feed) -> Self {
        let rss_entries = feed.entries.into_iter().map(RssEntry::from).collect();
        let mut new_rss_feed = RssFeed {
            id: feed.id,
            title: feed
                .title
                .map(|t| t.content)
                .unwrap_or_else(|| "Untitled".into()),
            link: feed
                .links
                .first()
                .map(|l| l.href.clone())
                .unwrap_or_default(),
            rss_entries,
            expanded: false,
        };
        new_rss_feed
            .rss_entries
            .sort_by_key(|e| Reverse(e.published));
        new_rss_feed
    }
}

#[derive(Clone)]
pub struct RssEntry {
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub content: String,
    pub link: String,
    pub published: Option<DateTime<Utc>>,
    pub read: bool,
}

impl From<feed_rs::model::Entry> for RssEntry {
    fn from(entry: feed_rs::model::Entry) -> Self {
        let authors = entry.authors.into_iter().map(|a| a.name).collect();
        let content = entry
            .content
            .and_then(|c| {
                let parsed_html = from_read(c.body?.clone().as_bytes(), usize::MAX)
                    .expect("Failed to parse HTML");
                return Some(parsed_html);
            })
            .or_else(|| {
                entry.summary.map(|s| {
                    let formatted_summary =
                        format!("{}\n\nFull content available online.", s.content);
                    return formatted_summary;
                })
            })
            .unwrap_or_default();

        RssEntry {
            id: entry.id,
            title: entry
                .title
                .map(|t| t.content)
                .unwrap_or_else(|| "Untitled".into()),
            authors,
            content: content,
            link: entry
                .links
                .first()
                .map(|l| l.href.clone())
                .unwrap_or_default(),
            published: entry.published,
            read: false,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
enum RunningState {
    #[default]
    Running,
    Done,
}

pub enum AppEvent {
    FeedFetched(Result<feed_rs::model::Feed, String>),
    ScrapedEntry {
        rss_feed_index: usize,
        rss_entry_index: usize,
        result: Result<String, String>,
    },
}

pub struct App {
    // For asynchronous events, like synchronizing feeds
    // or adding a new feed.
    pub sender: mpsc::UnboundedSender<AppEvent>,
    pub error_message: Option<String>,
    // Position of the cursor in the input field.
    pub character_index: usize,
    // Which screen to display.
    pub view_state: ViewState,
    // Which popup to display.
    pub popup: PopupState,
    // User input. For example, when adding a new feed.
    pub input: String,
    // The position of the cursor in the feeds list.
    pub cursor: usize,
    // Feeds, which contain entries.
    pub rss_feeds: Vec<RssFeed>,
    // Current running state.
    pub running_state: RunningState,
    pub scrollbar_state: ScrollbarState,
    // The current visual line for the current article.
    pub rss_entry_scroll: u16,
    // Previous entry area. Used for visual navigation.
    pub last_rss_entry_area: Rect,
}

impl App {
    pub fn new(sender: mpsc::UnboundedSender<AppEvent>) -> App {
        App {
            sender: sender,
            error_message: None,
            character_index: 0,
            view_state: ViewState::RssFeeds,
            popup: PopupState::None,
            input: String::new(),
            cursor: 0,
            rss_feeds: vec![],
            running_state: RunningState::Running,
            scrollbar_state: ScrollbarState::new(0),
            rss_entry_scroll: 0,
            last_rss_entry_area: Rect::default(),
        }
    }

    pub fn get_max_rss_entry_scroll(
        &self,
        rss_feed_index: usize,
        rss_entry_index: usize,
        area_height: u16,
    ) -> u16 {
        let content = &self.rss_feeds[rss_feed_index].rss_entries[rss_entry_index].content;
        let total_lines = content.len() as i32;
        let visible_lines = area_height as i32;
        let max_scroll = total_lines - visible_lines;
        if max_scroll < 0 { 0 } else { max_scroll as u16 }
    }

    pub fn add_rss_feed(&mut self) {
        let rss_feed_url: String = self.input.clone();
        self.input.clear();
        self.reset_cursor();
        let sender = self.sender.clone();

        // Use a background thread to retrieve the new feed.
        tokio::spawn(async move {
            let result = async {
                let rss_body = reqwest::get(&rss_feed_url)
                    .await
                    .map_err(|e| format!("Failed to add feed: {}", e.to_string()))?
                    .text()
                    .await
                    .map_err(|e| format!("Failed to add feed: {}", e.to_string()))?;

                let feed = feed_rs::parser::parse(rss_body.as_bytes())
                    .map_err(|e| format!("Failed to add feed: {}", e.to_string()))?;
                Ok(feed)
            }
            .await;
            let _ = sender.send(AppEvent::FeedFetched(result));
        });
    }

    pub fn delete_rss_feed(&mut self, rss_feed_index: usize) {
        self.rss_feeds.remove(rss_feed_index);
    }

    // Cursor methods are from the ratatui user input sample:
    // https://ratatui.rs/examples/apps/user_input/.

    pub fn move_cursor_left(&mut self) {
        let cursor_moved_left = self.character_index.saturating_sub(1);
        self.character_index = self.clamp_cursor(cursor_moved_left);
    }

    pub fn move_cursor_right(&mut self) {
        let cursor_moved_right = self.character_index.saturating_add(1);
        self.character_index = self.clamp_cursor(cursor_moved_right);
    }

    pub fn enter_char(&mut self, new_char: char) {
        let index = self.byte_index();
        self.input.insert(index, new_char);
        self.move_cursor_right();
    }

    /// Returns the byte index based on the character position.
    ///
    /// Since each character in a string can be contain multiple bytes, it's necessary to calculate
    /// the byte index based on the index of the character.
    fn byte_index(&self) -> usize {
        self.input
            .char_indices()
            .map(|(i, _)| i)
            .nth(self.character_index)
            .unwrap_or(self.input.len())
    }
    pub fn delete_char(&mut self) {
        let is_not_cursor_leftmost = self.character_index != 0;
        if is_not_cursor_leftmost {
            // Method "remove" is not used on the saved text for deleting the selected char.
            // Reason: Using remove on String works on bytes instead of the chars.
            // Using remove would require special care because of char boundaries.

            let current_index = self.character_index;
            let from_left_to_current_index = current_index - 1;

            // Getting all characters before the selected character.
            let before_char_to_delete = self.input.chars().take(from_left_to_current_index);
            // Getting all characters after selected character.
            let after_char_to_delete = self.input.chars().skip(current_index);

            // Put all characters together except the selected one.
            // By leaving the selected one out, it is forgotten and therefore deleted.
            self.input = before_char_to_delete.chain(after_char_to_delete).collect();
            self.move_cursor_left();
        }
    }

    fn clamp_cursor(&self, new_cursor_pos: usize) -> usize {
        new_cursor_pos.clamp(0, self.input.chars().count())
    }

    fn reset_cursor(&mut self) {
        self.character_index = 0;
    }
}
