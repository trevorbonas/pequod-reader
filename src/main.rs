use std::cmp::PartialEq;

use anyhow::{Result, anyhow};

use chrono::{DateTime, Utc};

use crossterm::event::{self, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use ratatui::Terminal;
use ratatui::layout::{Constraint, Direction, Layout, Margin};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Borders, List, ListItem, Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::{
    Frame,
    layout::Rect,
    style::Stylize,
    text::Line,
    widgets::{Block, Paragraph},
};

mod tui;

struct Feed {
    name: String,
    articles: Vec<Article>,
    expanded: bool,
}

#[derive(Clone)]
struct Article {
    title: String,
    authors: Vec<String>,
    content: String,
    read: bool,
    date: DateTime<Utc>,
}

enum Row {
    Feed(usize),           // Feed index.
    Article(usize, usize), // Feed index and article index.
}

#[derive(Debug, Default, PartialEq, Eq)]
enum ViewState {
    #[default]
    Feeds,
    Article {
        feed_index: usize,
        article_index: usize,
    },
}

#[derive(Debug, Default, PartialEq, Eq)]
enum RunningState {
    #[default]
    Running,
    Done,
}

struct App {
    view_state: ViewState,
    cursor: usize,
    feeds: Vec<Feed>,
    running_state: RunningState,
    scrollbar_state: ScrollbarState,
    article_scroll: u16,
    last_article_area: Rect,
}

impl App {
    pub fn new() -> App {
        App {
            view_state: ViewState::Feeds,
            cursor: 0,
            feeds: vec![],
            running_state: RunningState::Running,
            scrollbar_state: ScrollbarState::new(0),
            article_scroll: 0,
            last_article_area: Rect::default(),
        }
    }

    pub fn article_max_scroll(
        &self,
        feed_index: usize,
        article_index: usize,
        area_height: u16,
        area_width: u16,
    ) -> u16 {
        let content = &self.feeds[feed_index].articles[article_index].content;
        let wrapped_lines = wrap_text_lines(content.as_str(), area_width as usize);
        let total_lines = wrapped_lines.len() as i32;
        let visible_lines = area_height as i32;
        let max_scroll = total_lines - visible_lines;
        if max_scroll < 0 { 0 } else { max_scroll as u16 }
    }
}

fn wrap_text_lines(text: &str, area_width: usize) -> Vec<String> {
    textwrap::wrap(text, area_width)
        .into_iter()
        .map(|c| c.into_owned())
        .collect()
}

fn run_app<B: ratatui::backend::Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<()> {
    loop {
        terminal
            .draw(|f| ui(app, f))
            .map_err(|e| anyhow!("Failed to draw: {}", e))?;

        let rows = get_rows(app);
        if event::poll(std::time::Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                match app.view_state {
                    ViewState::Feeds => match key.code {
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
                        KeyCode::Esc | KeyCode::Char('q') => return Ok(()),
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
                        KeyCode::Char('c') => match rows[app.cursor] {
                            Row::Feed(feed_index) => {
                                app.feeds[feed_index].expanded = false;
                            }
                            Row::Article(feed_index, _) => {
                                app.feeds[feed_index].expanded = false;
                                app.cursor = feed_index;
                            }
                        },
                        KeyCode::Enter => match rows[app.cursor] {
                            Row::Feed(feed_index) => {
                                app.feeds[feed_index].expanded = !app.feeds[feed_index].expanded;
                                app.cursor = feed_index;
                            }
                            Row::Article(feed_index, article_index) => {
                                app.feeds[feed_index].articles[article_index].read = true;
                                app.view_state = ViewState::Article {
                                    feed_index: feed_index,
                                    article_index: article_index,
                                };
                            }
                        },
                        _ => {}
                    },
                    ViewState::Article {
                        feed_index,
                        article_index,
                    } => match key.code {
                        KeyCode::Esc | KeyCode::Char('q') => {
                            app.view_state = ViewState::Feeds;
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if app.article_scroll > 0 {
                                app.article_scroll -= 1;
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            let area = app.last_article_area;
                            let max_article_scroll = app.article_max_scroll(
                                feed_index,
                                article_index,
                                area.height,
                                area.width,
                            );
                            if app.article_scroll < max_article_scroll {
                                app.article_scroll += 1;
                            }
                        }
                        KeyCode::End | KeyCode::Char('G') => {
                            let area = app.last_article_area;
                            let max_scroll = app.article_max_scroll(
                                feed_index,
                                article_index,
                                area.height,
                                area.width,
                            );
                            app.article_scroll = max_scroll;
                        }
                        _ => {}
                    },
                }
            }
        }
    }
}

fn ui(app: &mut App, frame: &mut Frame) {
    match app.view_state {
        ViewState::Feeds => draw_list(frame, app),
        ViewState::Article {
            feed_index,
            article_index,
        } => draw_article(frame, app, feed_index, article_index),
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
            Row::Feed(feed_index) => {
                let feed = &app.feeds[*feed_index];
                let num_unread_articles = feed.articles.iter().filter(|a| a.read == false).count();
                let num_unread_articles_formatted = format!(" {}*", num_unread_articles);
                let postfix = if num_unread_articles == 0 {
                    ""
                } else {
                    &num_unread_articles_formatted
                };
                let prefix = if feed.expanded { "▼ " } else { "▶ " };
                ListItem::new(format!("{}{}{}", prefix, feed.name, postfix))
            }
            Row::Article(feed_index, article_index) => {
                let article = &app.feeds[*feed_index].articles[*article_index];
                let article_postfix = if article.read { "" } else { "*" };
                ListItem::new(format!("    {}{}", article.title, article_postfix))
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
        "Sync".into(),
        "<s> ".blue().bold().into(),
        "Quit".into(),
        "<q> ".blue().bold().into(),
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
        &mut app.scrollbar_state.clone(),
    );
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_article(
    frame: &mut ratatui::Frame,
    app: &mut App,
    feed_index: usize,
    article_index: usize,
) {
    let size = frame.area();
    app.last_article_area = size;
    let article = &app.feeds[feed_index].articles[article_index];
    let instructions = Line::from(vec![
        " ↓".into(),
        "<j> ".blue().bold().into(),
        "↑".into(),
        "<k> ".blue().bold().into(),
        "Top".into(),
        "<gg> ".blue().bold().into(),
        "Bottom".into(),
        "<G> ".blue().bold().into(),
        "Help".into(),
        "<h> ".blue().bold().into(),
        "Back".into(),
        "<q> ".blue().bold().into(),
    ]);
    let paragraph = Paragraph::new(article.content.clone())
        .block(
            Block::default()
                .title(article.title.clone())
                .title_bottom(instructions.centered())
                .borders(Borders::ALL),
        )
        .wrap(ratatui::widgets::Wrap { trim: true })
        .scroll((app.article_scroll, 0));
    frame.render_widget(paragraph, size);
}

fn get_rows(app: &App) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();

    for (feed_index, feed) in app.feeds.iter().enumerate() {
        rows.push(Row::Feed(feed_index));
        if feed.expanded {
            for (article_index, _) in feed.articles.iter().enumerate() {
                rows.push(Row::Article(feed_index, article_index));
            }
        }
    }
    rows
}

fn main() -> Result<()> {
    enable_raw_mode()?;
    let mut terminal = ratatui::init();
    let mut app = App::new();
    app.feeds = vec![Feed {
        name: "BBC".into(),
        expanded: false,
        articles: vec![
            Article {
                title: "This one".into(),
                authors: vec!["Someone".into()],
                content: "Some text for someone to read and other stuff . . .".into(),
                date: chrono::offset::Utc::now(),
                read: false,
            },
            Article {
                title: "Another one".into(),
                authors: vec!["Someone".into()],
                content: "Some text for someone to read and other stuff . . .".into(),
                date: chrono::offset::Utc::now(),
                read: false,
            },
            Article {
                title: "Another one".into(),
                authors: vec!["Someone".into()],
                content: "Some text for someone to read and other stuff . . .".into(),
                date: chrono::offset::Utc::now(),
                read: false,
            },
            Article {
                title: "Another one".into(),
                authors: vec!["Someone".into()],
                content: "Some text for someone to read and other stuff . . .".into(),
                date: chrono::offset::Utc::now(),
                read: false,
            },
        ],
    }];
    let _ = run_app(&mut terminal, &mut app);
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        ratatui::crossterm::event::DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}
