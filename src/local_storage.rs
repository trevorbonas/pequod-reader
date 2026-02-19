//! Local storage that contains RSS feed data.

use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, params};

use crate::app::{RssEntry, RssFeed};

/// Handles saving to and loading from a local
/// SQLite database.
pub struct LocalStorage {
    pub conn: Connection,
    pub max_ttl: Duration,
}

impl LocalStorage {
    pub fn new(db_path: PathBuf, max_ttl: Duration) -> rusqlite::Result<Self> {
        let conn = Connection::open(db_path.clone())?;
        Self::init(&conn)?;
        Ok(Self { conn, max_ttl })
    }

    /// Creates tables if they don't already exist.
    fn init(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute_batch(
            r#"
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS rss_feeds (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                link TEXT NOT NULL,
                expanded INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS rss_entries (
                id TEXT PRIMARY KEY,
                rss_feed_id TEXT NOT NULL,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                content_total_lines INTEGER NOT NULL,
                link TEXT NOT NULL,
                published TEXT NOT NULL,
                read INTEGER NOT NULL,
                authors TEXT,
                FOREIGN KEY(rss_feed_id) REFERENCES rss_feeds(id) ON DELETE CASCADE
            )
            "#,
        )?;
        Ok(())
    }

    /// Persists a single RSS entry.
    pub fn save_rss_entry(
        &mut self,
        rss_feed_id: &String,
        rss_entry: &RssEntry,
    ) -> rusqlite::Result<()> {
        let transaction = self.conn.transaction()?;
        let authors_json =
            serde_json::to_string(&rss_entry.authors).expect("authors failed to serialize");
        transaction.execute(
            "INSERT OR REPLACE INTO rss_entries
            (id, rss_feed_id, title, authors, content, content_total_lines,
             link, published, read)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                rss_entry.id,
                rss_feed_id,
                rss_entry.title,
                authors_json,
                rss_entry.content,
                rss_entry.content_total_lines as i64,
                rss_entry.link,
                rss_entry.published.to_rfc3339(),
                rss_entry.read as i32
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Saves an RSS feed and all of its entries.
    pub fn save_rss_feed(&mut self, rss_feed: &RssFeed) -> rusqlite::Result<()> {
        let transaction = self.conn.transaction()?;
        transaction.execute(
            "INSERT OR REPLACE INTO rss_feeds (id, title, link, expanded)
            VALUES(?1, ?2, ?3, ?4)",
            params![
                rss_feed.id,
                rss_feed.title,
                rss_feed.link,
                rss_feed.expanded as i32
            ],
        )?;

        for rss_entry in &rss_feed.rss_entries {
            let authors_json =
                serde_json::to_string(&rss_entry.authors).expect("authors failed to serialize");
            transaction.execute(
                "INSERT OR REPLACE INTO rss_entries
                (id, rss_feed_id, title, authors, content, content_total_lines,
                 link, published, read)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    rss_entry.id,
                    rss_feed.id,
                    rss_entry.title,
                    authors_json,
                    rss_entry.content,
                    rss_entry.content_total_lines as i64,
                    rss_entry.link,
                    rss_entry.published.to_rfc3339(),
                    rss_entry.read as i32
                ],
            )?;
        }

        transaction.commit()?;
        Ok(())
    }

    /// Saves multiple RSS feeds and all of their entries.
    pub fn save_rss_feeds(&mut self, rss_feeds: &Vec<RssFeed>) -> rusqlite::Result<()> {
        for rss_feed in rss_feeds {
            self.save_rss_feed(rss_feed)?;
        }
        Ok(())
    }

    /// Loads all available RSS feeds and translates rows to RssFeeds.
    pub fn load_rss_feeds(&self) -> rusqlite::Result<Vec<RssFeed>> {
        let mut rss_feed_statement = self
            .conn
            .prepare("SELECT id, title, link, expanded FROM rss_feeds ORDER BY title ASC")?;
        let rss_feed_rows = rss_feed_statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i32>(3)?,
            ))
        })?;

        let mut rss_feeds: Vec<RssFeed> = Vec::new();

        for rss_feed_row in rss_feed_rows {
            let (id, title, link, expanded) = rss_feed_row?;
            let rss_entries = self.load_rss_entries_for_rss_feed(&id)?;
            rss_feeds.push(RssFeed {
                id,
                title,
                link,
                rss_entries,
                expanded: expanded != 0,
            });
        }
        Ok(rss_feeds)
    }

    pub fn load_rss_entries_for_rss_feed(
        &self,
        rss_feed_id: &String,
    ) -> rusqlite::Result<Vec<RssEntry>> {
        let mut statement = self.conn.prepare(
            "SELECT id, title, authors, content, content_total_lines, link,
                 published, read FROM rss_entries WHERE rss_feed_id = ?1 ORDER BY published DESC",
        )?;

        let rows = statement.query_map([rss_feed_id], |row| {
            let authors_json: String = row.get(2)?;
            let authors: Vec<String> = serde_json::from_str(&authors_json).unwrap_or_default();
            let published: String = row.get(6)?;
            let published = DateTime::parse_from_rfc3339(&published)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_default();

            Ok(RssEntry {
                id: row.get(0)?,
                title: row.get(1)?,
                authors,
                content: row.get(3)?,
                content_total_lines: row.get::<_, i64>(4)? as usize,
                link: row.get(5)?,
                published,
                read: row.get::<_, i32>(7)? != 0,
            })
        })?;

        let mut rss_entries: Vec<RssEntry> = Vec::new();
        for rss_entry in rows {
            rss_entries.push(rss_entry?);
        }

        Ok(rss_entries)
    }

    /// Deletes an RSS feed.
    pub fn delete_rss_feed(&self, rss_feed_id: &String) -> rusqlite::Result<usize> {
        let affected = self
            .conn
            .execute("DELETE FROM rss_feeds WHERE id = ?1", params![rss_feed_id])?;
        Ok(affected)
    }

    /// Removes old entries if they are unread.
    pub fn expire_old_entries(&self) -> rusqlite::Result<usize> {
        let cutoff = Utc::now() - self.max_ttl;
        let affected = self.conn.execute(
            "DELETE FROM entries WHERE published < ?1",
            [cutoff.to_rfc3339()],
        )?;
        Ok(affected)
    }
}
