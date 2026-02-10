use anyhow::{Result, anyhow};
use chrono::{DateTime, Local, Utc};
use crossterm::event::{self, Event, KeyCode, KeyEvent};
use crossterm::execute;
use crossterm::terminal::{LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use html2text::from_read;
use ratatui::Terminal;
use ratatui::layout::{Constraint, Direction, Flex, Layout, Margin, Position};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Span, Text};
use ratatui::widgets::{
    Borders, Clear, List, ListItem, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
};
use ratatui::{
    Frame,
    layout::Rect,
    style::Stylize,
    text::Line,
    widgets::{Block, Paragraph},
};
use std::char;
use std::cmp::{PartialEq, Reverse};
use textwrap::{Options, wrap};
use tokio::sync::mpsc;
use unicode_width::{self, UnicodeWidthChar, UnicodeWidthStr};

mod tui;

struct RssFeed {
    id: String,
    title: String,
    link: String,
    rss_entries: Vec<RssEntry>,
    expanded: bool,
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
struct RssEntry {
    id: String,
    title: String,
    authors: Vec<String>,
    lines: Vec<String>, // Pre-wrapped content.
    link: String,
    published: Option<DateTime<Utc>>,
    read: bool,
}

impl From<feed_rs::model::Entry> for RssEntry {
    fn from(entry: feed_rs::model::Entry) -> Self {
        let authors = entry.authors.into_iter().map(|a| a.name).collect();
        let lines = entry
            .content
            .and_then(|c| {
                let parsed_html =
                    from_read(c.body?.clone().as_bytes(), 50).expect("Failed to parse HTML");
                return Some(wrap_text_to_lines(&parsed_html, 50));
            })
            .or_else(|| {
                entry.summary.map(|s| {
                    let formatted_summary =
                        format!("{}\n\nFull content available online.", s.content);
                    return wrap_text_to_lines(&formatted_summary, 50);
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
            lines,
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

enum Row {
    RssFeed(usize),         // Feed index.
    RssEntry(usize, usize), // Feed index and entry index.
}

#[derive(Debug, Default, PartialEq, Eq)]
enum ViewState {
    #[default]
    // A list of feeds with nested entries.
    RssFeeds,
    // An entry title with a date and author(s).
    RssEntry {
        rss_feed_index: usize,
        rss_entry_index: usize,
    },
}

enum PopupState {
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

#[derive(Debug, Default, PartialEq, Eq)]
enum RunningState {
    #[default]
    Running,
    Done,
}

enum AppEvent {
    FeedFetched(Result<feed_rs::model::Feed, String>),
    ScrapedEntry {
        rss_feed_index: usize,
        rss_entry_index: usize,
        result: Result<String, String>,
    },
}

struct App {
    // For asynchronous events, like synchronizing feeds
    // or adding a new feed.
    sender: mpsc::UnboundedSender<AppEvent>,
    error_message: Option<String>,
    // Position of the cursor in the input field.
    character_index: usize,
    // Which screen to display.
    view_state: ViewState,
    // Which popup to display.
    popup: PopupState,
    // User input. For example, when adding a new feed.
    input: String,
    // The position of the cursor in the feeds list.
    cursor: usize,
    // Feeds, which contain entries.
    rss_feeds: Vec<RssFeed>,
    // Current running state.
    running_state: RunningState,
    scrollbar_state: ScrollbarState,
    // The current visual line for the current article.
    rss_entry_scroll: u16,
    // Previous entry area. Used for visual navigation.
    last_rss_entry_area: Rect,
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
        let lines = &self.rss_feeds[rss_feed_index].rss_entries[rss_entry_index].lines;
        let total_lines = lines.len() as i32;
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

    fn delete_rss_feed(&mut self, rss_feed_index: usize) {
        self.rss_feeds.remove(rss_feed_index);
    }

    // Cursor methods are from the ratatui user input sample:
    // https://ratatui.rs/examples/apps/user_input/.

    fn move_cursor_left(&mut self) {
        let cursor_moved_left = self.character_index.saturating_sub(1);
        self.character_index = self.clamp_cursor(cursor_moved_left);
    }

    fn move_cursor_right(&mut self) {
        let cursor_moved_right = self.character_index.saturating_add(1);
        self.character_index = self.clamp_cursor(cursor_moved_right);
    }

    fn enter_char(&mut self, new_char: char) {
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
    fn delete_char(&mut self) {
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
                let lines = wrap_text_to_lines(&content, app.last_rss_entry_area.width as usize);
                app.rss_feeds[rss_feed_index].rss_entries[rss_entry_index].lines = lines;
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

fn ui(app: &mut App, frame: &mut Frame) {
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
                let read_postfix = if rss_entry.read { "" } else { "*" };
                let text_to_wrap = format!(
                    "    {}{} - {}",
                    rss_entry.title,
                    read_postfix,
                    rss_entry
                        .published
                        .unwrap_or_default()
                        .with_timezone(&Local)
                        .format("%Y-%m-%d %I:%M%P")
                );
                let wrapped_lines: Vec<Line> = wrap(&text_to_wrap, area.width as usize)
                    .into_iter()
                    .map(|cow| Line::from(cow.into_owned()))
                    .collect();
                let text = Text::from(wrapped_lines);
                ListItem::new(text)
            }
        })
        .collect();

    let instructions = Line::from(vec![
        " ↓".into(),
        "<j> ".light_blue().bold().into(),
        "↑".into(),
        "<k> ".light_blue().bold().into(),
        "Select".into(),
        "<Enter> ".light_blue().bold().into(),
        "Add".into(),
        "<a> ".light_blue().bold().into(),
        "Delete".into(),
        "<d> ".light_blue().bold().into(),
        "Sync".into(),
        "<s> ".light_blue().bold().into(),
        "Quit".into(),
        "<q> ".light_blue().bold().into(),
    ]);

    let list = List::new(items)
        .block(
            Block::default()
                .title("Feeds".green().bold())
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

fn truncate_rss_entry_title(title: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(title) <= max_width {
        return title.to_string();
    }

    if max_width <= 3 {
        return ".".repeat(max_width);
    }
    let mut result = String::new();
    let mut width = 0;
    for ch in title.chars() {
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
        "<j> ".light_blue().bold().into(),
        "↑".into(),
        "<k> ".light_blue().bold().into(),
        "Fetch".into(),
        "<f> ".light_blue().bold().into(),
        "Open".into(),
        "<o> ".light_blue().bold().into(),
        "Help".into(),
        "<h> ".light_blue().bold().into(),
        "Back".into(),
        "<q> ".light_blue().bold().into(),
    ]);
    let visible_lines = rss_entry
        .lines
        .iter()
        .skip(app.rss_entry_scroll as usize)
        .take(size.height as usize);
    let text = visible_lines
        .map(|l| Line::from(l.clone()))
        .collect::<Vec<_>>();
    let paragraph = Paragraph::new(text).block(
        Block::default()
            .title(rss_entry.title.clone())
            .title_bottom(instructions.centered())
            .borders(Borders::ALL),
    );
    frame.render_widget(paragraph, size);
}

fn get_rows(app: &App) -> Vec<Row> {
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
    let instructions = Line::from(vec![" Back".into(), "<q> ".light_blue().bold().into()]);
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
    let instructions = Line::from(vec![" Back".into(), "<q> ".light_blue().bold().into()]);
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
        "<Enter> ".light_blue().bold().into(),
        "Back".into(),
        "<q> ".light_blue().bold().into(),
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
        "<y> ".light_blue().bold().into(),
        "No".into(),
        "<n> ".light_blue().bold().into(),
        "Cancel".into(),
        "<q> ".light_blue().bold().into(),
    ]);

    let horizontal = Layout::horizontal([Constraint::Percentage(85)]).flex(Flex::Center);
    let [popup_area] = horizontal.areas(area);
    let text_width = popup_area.width;
    let text = format!(
        "Are you sure that you want to delete feed \"{}\"",
        rss_feed_name
    );
    let wrapped_text = wrap_text_to_lines(&text, text_width as usize);
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
    let instructions = Line::from(vec![" Ok".into(), "<Enter> ".light_blue().bold().into()]);
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

fn wrap_text_to_lines(text: &str, width: usize) -> Vec<String> {
    wrap(text, width)
        .into_iter()
        .map(|l| l.to_string())
        .collect()
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
