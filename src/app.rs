use anyhow::Result;
use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use html2text::from_read;
use ratatui::layout::Rect;
use ratatui::widgets::ScrollbarState;
use std::char;
use std::cmp::{PartialEq, Reverse};
use tokio::sync::mpsc;

use crate::tui::{PopupState, Row, SPINNER_CHARS, ViewState};

#[derive(Clone)]
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
    pub content_total_lines: usize,
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
                let parsed_html =
                    from_read(c.body?.clone().as_bytes(), usize::MAX).unwrap_or_default();
                return Some(parsed_html);
            })
            .or_else(|| {
                entry.summary.map(|s| {
                    let parsed_html =
                        from_read(s.content.as_bytes(), usize::MAX).unwrap_or_default();
                    return parsed_html;
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
            content_total_lines: 0, // All text is currently on a single line.
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

pub enum AppEvent {
    FeedFetched(Result<feed_rs::model::Feed, String>, String),
    ScrapedEntry {
        rss_feed_index: usize,
        rss_entry_index: usize,
        result: Result<String, String>,
    },
    SyncFinished(Result<Vec<RssFeed>, anyhow::Error>),
}

pub struct App {
    // For asynchronous events, like synchronizing feeds
    // or adding a new feed.
    pub sender: mpsc::UnboundedSender<AppEvent>,
    pub error_message: Option<String>,
    // Position of the cursor in the input field.
    pub character_index: usize,
    // The last key that was pressed.
    pub last_key: Option<KeyCode>,
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
    pub scrollbar_state: ScrollbarState,
    // The current visual line for the current article.
    pub rss_entry_scroll: u16,
    // Previous frame area. Used for visual navigation.
    pub last_frame_area: Rect,
    pub syncing: bool,
    pub spinner_index: usize,
}

impl App {
    pub fn new(sender: mpsc::UnboundedSender<AppEvent>) -> App {
        App {
            sender: sender,
            error_message: None,
            character_index: 0,
            last_key: None,
            view_state: ViewState::RssFeeds,
            popup: PopupState::None,
            input: String::new(),
            cursor: 0,
            rss_feeds: vec![],
            scrollbar_state: ScrollbarState::new(0),
            rss_entry_scroll: 0,
            last_frame_area: Rect::default(),
            syncing: false,
            spinner_index: 0,
        }
    }

    pub fn get_max_rss_entry_scroll(
        &self,
        rss_feed_index: usize,
        rss_entry_index: usize,
        area_height: u16,
    ) -> u16 {
        let content_total_lines = (self.rss_feeds[rss_feed_index].rss_entries[rss_entry_index]
            .content_total_lines) as u16;
        if content_total_lines < area_height {
            0
        } else {
            content_total_lines - area_height
        }
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
            let _ = sender.send(AppEvent::FeedFetched(result, rss_feed_url));
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

    fn sync(&mut self) {
        let sender = self.sender.clone();
        let rss_feeds = self.rss_feeds.clone();
        tokio::spawn(async move {
            let result = sync_feeds(rss_feeds).await;
            let _ = sender.send(AppEvent::SyncFinished(result));
        });
    }

    pub fn on_tick(&mut self) {
        if self.syncing {
            self.spinner_index = (self.spinner_index + 1) % SPINNER_CHARS.len();
        }
    }

    fn fetch_full_rss_entry_content(&mut self, rss_feed_index: usize, rss_entry_index: usize) {
        let sender = self.sender.clone();
        let link = self.rss_feeds[rss_feed_index].rss_entries[rss_entry_index]
            .link
            .clone();

        let html_width = self.last_frame_area.width;
        tokio::spawn(async move {
            let result = async {
                let html = reqwest::get(&link)
                    .await
                    .map_err(|e| format!("Failed to load full content: {}", e.to_string()))?
                    .text()
                    .await
                    .map_err(|e| format!("Failed to load full content: {}", e.to_string()))?;
                let parsed_html =
                    from_read(html.as_bytes(), html_width as usize).expect("Failed to parse HTML");
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

    pub fn handle_app_event(&mut self, app_event: AppEvent) {
        match app_event {
            AppEvent::ScrapedEntry {
                rss_feed_index,
                rss_entry_index,
                result,
            } => match result {
                Ok(content) => {
                    self.rss_feeds[rss_feed_index].rss_entries[rss_entry_index].content = content;
                    self.error_message = None;
                }
                Err(err) => {
                    self.error_message = Some(err);
                }
            },
            AppEvent::FeedFetched(Ok(feed), feed_url) => {
                let mut new_rss_feed = RssFeed::from(feed);
                new_rss_feed.link = feed_url;
                self.rss_feeds.push(new_rss_feed);
                self.error_message = None;
            }
            AppEvent::FeedFetched(Err(err), _) => {
                self.error_message = Some(err);
                self.popup = PopupState::Error;
            }
            AppEvent::SyncFinished(result) => {
                self.popup = PopupState::None;
                self.syncing = false;
                match result {
                    Ok(rss_feeds) => {
                        self.rss_feeds = rss_feeds;
                    }
                    Err(e) => {
                        self.error_message = Some(format!("Sync failed: {}", e));
                    }
                }
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent, rows: &[Row]) -> Result<bool> {
        match self.popup {
            PopupState::AddRssFeed => self.handle_add_rss_feed_popup(key),
            PopupState::ConfirmDeleteRssFeed => self.handle_delete_rss_feed_popup(key, rows),
            PopupState::Error => self.handle_error_popup(key),
            PopupState::RssEntryHelp => self.handle_rss_entry_help_popup(key),
            PopupState::RssFeedHelp => self.handle_rss_feed_help_popup(key),
            PopupState::None => self.handle_default(key, rows),
            PopupState::Syncing => Ok(false),
        }
    }

    fn handle_add_rss_feed_popup(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.input.clear();
                self.character_index = 0;
                self.popup = PopupState::None;
            }
            KeyCode::Enter => {
                self.add_rss_feed();
                self.input.clear();
                self.character_index = 0;
                self.popup = PopupState::None;
            }
            KeyCode::Char(c) => self.enter_char(c),
            KeyCode::Backspace => self.delete_char(),
            KeyCode::Left => self.move_cursor_left(),
            KeyCode::Right => self.move_cursor_right(),
            _ => {}
        }
        Ok(false)
    }

    fn handle_delete_rss_feed_popup(&mut self, key: KeyEvent, rows: &[Row]) -> Result<bool> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('n') => self.popup = PopupState::None,
            KeyCode::Char('y') => {
                let row = &rows[self.cursor];
                match row {
                    Row::RssFeed(rss_feed_index) => {
                        self.delete_rss_feed(*rss_feed_index);
                        if *rss_feed_index > 0 {
                            self.cursor = rss_feed_index - 1;
                        } else {
                            self.cursor = 0;
                        }
                    }
                    Row::RssEntry(rss_feed_index, rss_entry_index) => {
                        self.delete_rss_feed(*rss_feed_index);
                        if *rss_feed_index > 0 {
                            self.cursor = self.cursor - rss_entry_index - 2;
                        } else {
                            self.cursor = 0;
                        }
                    }
                }
                self.popup = PopupState::None;
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_error_popup(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                self.error_message = None;
                self.popup = PopupState::None;
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_rss_feed_help_popup(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.popup = PopupState::None,
            _ => {}
        }
        Ok(false)
    }

    fn handle_rss_entry_help_popup(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.popup = PopupState::None,
            _ => {}
        }
        Ok(false)
    }

    fn handle_default(&mut self, key: KeyEvent, rows: &[Row]) -> Result<bool> {
        match self.view_state {
            ViewState::RssFeeds => self.handle_rss_feeds_view(key, rows),
            ViewState::RssEntry {
                rss_feed_index,
                rss_entry_index,
            } => self.handle_rss_entry_view(key, rss_feed_index, rss_entry_index),
        }
    }

    fn handle_rss_feeds_view(&mut self, key: KeyEvent, rows: &[Row]) -> Result<bool> {
        match key.code {
            KeyCode::Char('s') => {
                self.popup = PopupState::Syncing;
                self.syncing = true;
                self.sync();
            }
            KeyCode::Char('g') => {
                if self.last_key == Some(KeyCode::Char('g')) {
                    self.cursor = 0;
                    self.last_key = None;
                } else {
                    self.last_key = Some(KeyCode::Char('g'));
                }
            }
            KeyCode::Char('G') => {
                self.last_key = Some(KeyCode::Char('G'));
                self.cursor = rows.len() - 1;
            }
            KeyCode::Esc => {
                self.last_key = Some(KeyCode::Esc);
                return Ok(true);
            }
            KeyCode::Char('q') => {
                self.last_key = Some(KeyCode::Char('q'));
                return Ok(true);
            }
            KeyCode::Down => {
                self.last_key = Some(KeyCode::Down);
                if self.cursor + 1 < rows.len() {
                    self.cursor += 1;
                }
            }
            KeyCode::Char('j') => {
                self.last_key = Some(KeyCode::Char('j'));
                if self.cursor + 1 < rows.len() {
                    self.cursor += 1;
                }
            }
            KeyCode::Up => {
                self.last_key = Some(KeyCode::Up);
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
            }
            KeyCode::Char('k') => {
                self.last_key = Some(KeyCode::Char('k'));
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
            }
            KeyCode::Char('a') => {
                self.last_key = Some(KeyCode::Char('a'));
                self.popup = PopupState::AddRssFeed;
            }
            KeyCode::Char('h') => {
                self.last_key = Some(KeyCode::Char('h'));
                self.popup = PopupState::RssFeedHelp;
            }
            KeyCode::Char('c') => {
                self.last_key = Some(KeyCode::Char('c'));
                match rows[self.cursor] {
                    Row::RssFeed(rss_feed_index) => {
                        self.rss_feeds[rss_feed_index].expanded = false;
                    }
                    Row::RssEntry(rss_feed_index, rss_entry_index) => {
                        self.rss_feeds[rss_feed_index].expanded = false;
                        self.cursor = self.cursor - rss_entry_index - 1;
                    }
                }
            }
            KeyCode::Enter => {
                self.last_key = Some(KeyCode::Enter);
                if rows.len() > 0 {
                    match rows[self.cursor] {
                        Row::RssFeed(rss_feed_index) => {
                            self.rss_feeds[rss_feed_index].expanded =
                                !self.rss_feeds[rss_feed_index].expanded;
                        }
                        Row::RssEntry(rss_feed_index, rss_entry_index) => {
                            self.rss_entry_scroll = 0;
                            self.rss_feeds[rss_feed_index].rss_entries[rss_entry_index].read = true;
                            self.view_state = ViewState::RssEntry {
                                rss_feed_index,
                                rss_entry_index,
                            };
                        }
                    }
                }
            }
            KeyCode::Char('u') => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    let half_page = (self.last_frame_area.height as usize - 2) / 2;
                    if self.cursor.saturating_sub(half_page) == 0 {
                        self.cursor = 0;
                    } else {
                        self.cursor = self.cursor.saturating_sub(half_page);
                    }
                } else {
                    self.last_key = Some(KeyCode::Char('u'));
                }
            }
            KeyCode::Char('d') => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    let half_page = (self.last_frame_area.height as usize - 2) / 2;
                    if self.cursor + half_page >= rows.len() {
                        self.cursor = rows.len() - 1;
                    } else {
                        self.cursor += half_page;
                    }
                } else {
                    self.last_key = Some(KeyCode::Char('d'));
                    match rows[self.cursor] {
                        Row::RssFeed(_) => {
                            self.popup = PopupState::ConfirmDeleteRssFeed;
                        }
                        Row::RssEntry(_, _) => {
                            self.popup = PopupState::ConfirmDeleteRssFeed;
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_rss_entry_view(
        &mut self,
        key: KeyEvent,
        rss_feed_index: usize,
        rss_entry_index: usize,
    ) -> Result<bool> {
        match key.code {
            KeyCode::Char('o') => {
                self.last_key = Some(KeyCode::Char('o'));
                open::that(
                    self.rss_feeds[rss_feed_index].rss_entries[rss_entry_index]
                        .link
                        .clone(),
                )?;
            }
            KeyCode::Char('f') => {
                self.last_key = Some(KeyCode::Char('f'));
                self.fetch_full_rss_entry_content(rss_feed_index, rss_entry_index);
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                self.last_key = Some(KeyCode::Char('q'));
                self.view_state = ViewState::RssFeeds;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.last_key = Some(KeyCode::Char('k'));
                if self.rss_entry_scroll > 0 {
                    self.rss_entry_scroll -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.last_key = Some(KeyCode::Char('j'));
                let area = self.last_frame_area;
                let max_rss_entry_scroll =
                    self.get_max_rss_entry_scroll(rss_feed_index, rss_entry_index, area.height);
                if self.rss_entry_scroll < max_rss_entry_scroll {
                    self.rss_entry_scroll += 1;
                }
            }
            KeyCode::Char('h') => {
                self.last_key = Some(KeyCode::Char('h'));
                self.popup = PopupState::RssEntryHelp;
            }
            KeyCode::Char('g') => {
                if self.last_key == Some(KeyCode::Char('g')) {
                    self.rss_entry_scroll = 0;
                    self.last_key = None;
                } else {
                    self.last_key = Some(KeyCode::Char('g'));
                }
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.last_key = Some(KeyCode::Char('G'));
                let area = self.last_frame_area;
                let max_scroll =
                    self.get_max_rss_entry_scroll(rss_feed_index, rss_entry_index, area.height);
                self.rss_entry_scroll = max_scroll;
            }
            KeyCode::Char('u') => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    let half_page = (self.last_frame_area.height - 2) / 2;
                    if self.rss_entry_scroll.saturating_sub(half_page) == 0 {
                        self.rss_entry_scroll = 0;
                    } else {
                        self.rss_entry_scroll = self.rss_entry_scroll.saturating_sub(half_page);
                    }
                } else {
                    self.last_key = Some(KeyCode::Char('u'));
                }
            }
            KeyCode::Char('d') => {
                let rss_entry = &self.rss_feeds[rss_feed_index].rss_entries[rss_entry_index];
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    let frame_height = self.last_frame_area.height;
                    let half_page = (self.last_frame_area.height - 2) / 2;
                    if self.rss_entry_scroll + half_page
                        >= (rss_entry.content_total_lines as u16).saturating_sub(frame_height)
                    {
                        self.rss_entry_scroll =
                            ((rss_entry.content_total_lines) as u16).saturating_sub(frame_height);
                    } else {
                        self.rss_entry_scroll += half_page;
                    }
                } else {
                    self.last_key = Some(KeyCode::Char('d'));
                }
            }
            _ => {}
        }
        Ok(false)
    }
}

/// Update a Vec<RssFeeds>, adding newer RSS entries.
async fn sync_feeds(mut rss_feeds: Vec<RssFeed>) -> Result<Vec<RssFeed>> {
    let client = reqwest::Client::new();
    for rss_feed in rss_feeds.iter_mut() {
        let newest_date = rss_feed
            .rss_entries
            .first()
            .and_then(|e| e.published)
            .unwrap_or_default();
        let response_text = client.get(&rss_feed.link).send().await?.text().await?;
        let updated_feed = feed_rs::parser::parse(response_text.as_bytes())?;
        for entry in updated_feed.entries {
            if entry.published.unwrap_or_default() > newest_date {
                let rss_entry = RssEntry::from(entry);
                rss_feed.rss_entries.push(rss_entry)
            }
        }
        rss_feed.rss_entries.sort_by_key(|e| Reverse(e.published));
    }
    Ok(rss_feeds)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::tui::PopupState;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tokio::time::timeout;

    #[tokio::test]
    async fn test_open_rss_entry() {
        let (sender, _) = mpsc::unbounded_channel();
        let rows: Vec<Row> = vec![Row::RssFeed(0), Row::RssEntry(0, 0)];
        let mut app = App::new(sender);
        // Last frame area will affect the outcome of attempting to scroll.
        // If this is left as its default, each 'j' key press will scroll
        // downwards, when, in this test, the entry content is very small.
        app.last_frame_area = Rect {
            x: 0,
            y: 0,
            width: 50,
            height: 30,
        };
        app.rss_feeds = vec![RssFeed {
            id: "rss-feed-test-id".to_string(),
            title: "rss feed test title".to_string(),
            rss_entries: vec![RssEntry {
                id: "rss-feed-test-id".to_string(),
                title: "rss entry test title".to_string(),
                authors: vec!["Test Person".to_string()],
                published: Some(chrono::offset::Utc::now()),
                content: "Test content.".to_string(),
                content_total_lines: 1,
                read: false,
                link: "https://example.com".to_string(),
            }],
            expanded: false,
            link: "https://example.com".to_string(),
        }];

        // Expand RSS feed.
        assert!(app.view_state == ViewState::RssFeeds);
        assert!(
            !app.rss_feeds
                .first()
                .unwrap()
                .rss_entries
                .first()
                .unwrap()
                .read
        );
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &rows)
            .unwrap();
        assert!(app.popup == PopupState::None);
        assert!(app.rss_feeds.first().unwrap().expanded);
        assert!(app.view_state == ViewState::RssFeeds);

        // Scroll down one row to the RSS entry and open it.
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE), &rows)
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &rows)
            .unwrap();
        assert!(
            app.view_state
                == ViewState::RssEntry {
                    rss_feed_index: 0,
                    rss_entry_index: 0
                }
        );
        assert!(
            app.rss_feeds
                .first()
                .unwrap()
                .rss_entries
                .first()
                .unwrap()
                .read
        );
        assert!(app.rss_entry_scroll == 0);
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE), &rows)
            .unwrap();
        assert!(app.rss_entry_scroll == 0);
        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE), &rows)
            .unwrap();
        assert!(app.view_state == ViewState::RssFeeds);
        let quit_result = app
            .handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE), &rows)
            .unwrap();
        // A true result means the application should exit.
        assert!(quit_result);
    }

    #[tokio::test]
    async fn test_delete_rss_feed() {
        let (sender, _) = mpsc::unbounded_channel();
        let rows: Vec<Row> = vec![Row::RssFeed(0), Row::RssEntry(0, 0)];
        let mut app = App::new(sender);
        // Last frame area will affect the outcome of attempting to scroll.
        // If this is left as its default, each 'j' key press will scroll
        // downwards, when, in this test, the entry content is very small.
        app.last_frame_area = Rect {
            x: 0,
            y: 0,
            width: 50,
            height: 30,
        };
        app.rss_feeds = vec![RssFeed {
            id: "rss-feed-test-id".to_string(),
            title: "rss feed test title".to_string(),
            rss_entries: vec![RssEntry {
                id: "rss-feed-test-id".to_string(),
                title: "rss entry test title".to_string(),
                authors: vec!["Test Person".to_string()],
                published: Some(chrono::offset::Utc::now()),
                content: "Test content.".to_string(),
                content_total_lines: 1,
                read: false,
                link: "https://example.com".to_string(),
            }],
            expanded: false,
            link: "https://example.com".to_string(),
        }];

        // Delete the RSS feed.
        assert!(app.view_state == ViewState::RssFeeds);
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE), &rows)
            .unwrap();
        // Confirm deletion by pressing 'y'.
        assert!(app.popup == PopupState::ConfirmDeleteRssFeed);
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE), &rows)
            .unwrap();

        assert!(app.popup == PopupState::None);
        assert!(app.rss_feeds.len() == 0);

        let quit_result = app
            .handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE), &rows)
            .unwrap();
        // A true result means the application should exit.
        assert!(quit_result);
    }

    #[tokio::test]
    async fn test_add_rss_feed_failure() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let rows: Vec<Row> = Vec::new();
        let mut app = App::new(sender);

        // Enter 'a', causing the "Add feed" popup to open.
        let add_key_event = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        app.handle_key(add_key_event, &rows).unwrap();
        assert!(app.popup == PopupState::AddRssFeed);

        // Enter mock RSS feed URL.
        app.input = "https://example.com/rss.xml".to_string();
        let enter_key_event = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        app.handle_key(enter_key_event, &rows).unwrap();
        assert!(app.popup == PopupState::None);

        let app_event = timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("timed out waiting for AppEvent")
            .expect("channel closed");
        app.handle_app_event(app_event);
        assert!(app.error_message.is_some());
        assert!(
            app.error_message.unwrap()
                == "Failed to add feed: unable to parse feed: no root element"
        );
        assert!(app.popup == PopupState::Error);
    }
}
