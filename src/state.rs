use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;
use std::fs;
use std::path::Path;

use crate::delivery::WakeEvent;
use crate::feed::FeedItem;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryStatus {
    Pending,
    Delivered,
}

impl DeliveryStatus {
    fn as_str(self) -> &'static str {
        match self {
            DeliveryStatus::Pending => "pending",
            DeliveryStatus::Delivered => "delivered",
        }
    }

    fn from_str(value: &str) -> Self {
        match value {
            "delivered" => DeliveryStatus::Delivered,
            _ => DeliveryStatus::Pending,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedCache {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

pub struct StateStore {
    conn: Connection,
}

impl StateStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create state dir {}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open state db {}", path.display()))?;
        let store = Self { conn };
        store.init()?;
        Ok(store)
    }

    pub fn memory() -> Result<Self> {
        let store = Self {
            conn: Connection::open_in_memory().context("failed to open in-memory state db")?,
        };
        store.init()?;
        Ok(store)
    }

    pub fn has_seen_url(&self, url: &str) -> Result<bool> {
        let exists: Option<i64> = self
            .conn
            .query_row(
                "SELECT 1 FROM seen_urls WHERE url = ?1",
                params![url],
                |row| row.get(0),
            )
            .optional()?;
        Ok(exists.is_some())
    }

    pub fn mark_seen_url(&self, url: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO seen_urls (url, first_seen_at) VALUES (?1, datetime('now'))",
            params![url],
        )?;
        Ok(())
    }

    pub fn enqueue_event(&self, event: &WakeEvent) -> Result<i64> {
        let payload = json!({
            "source": event.item.source_name,
            "sourceUrl": event.item.source_url,
            "title": event.item.title,
            "url": event.item.url,
            "description": event.item.description,
            "publishedAt": event.item.published_at.map(|value| value.to_rfc3339()),
            "matchedRule": event.matched_rule,
            "matchedEntity": event.matched_entity,
        })
        .to_string();

        self.conn.execute(
            "INSERT OR IGNORE INTO outbox_events
             (item_url, source_name, title, payload_json, matched_rule, matched_entity, status, attempts, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, datetime('now'))",
            params![
                event.item.url,
                event.item.source_name,
                event.item.title,
                payload,
                event.matched_rule,
                event.matched_entity,
                DeliveryStatus::Pending.as_str(),
            ],
        )?;

        let id = self.conn.query_row(
            "SELECT id FROM outbox_events WHERE item_url = ?1",
            params![event.item.url],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    pub fn pending_events(&self) -> Result<Vec<(i64, WakeEvent)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, source_name, title, item_url, payload_json, matched_rule, matched_entity
             FROM outbox_events WHERE status = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![DeliveryStatus::Pending.as_str()], |row| {
            let payload: String = row.get(4)?;
            let parsed: serde_json::Value =
                serde_json::from_str(&payload).unwrap_or(serde_json::Value::Null);
            let item = FeedItem {
                source_name: row.get(1)?,
                source_url: parsed
                    .get("sourceUrl")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string(),
                title: row.get(2)?,
                url: row.get(3)?,
                description: parsed
                    .get("description")
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned),
                published_at: None,
            };
            Ok((
                row.get(0)?,
                WakeEvent {
                    item,
                    matched_rule: row.get(5)?,
                    matched_entity: row.get(6)?,
                },
            ))
        })?;

        let mut events = Vec::new();
        for row in rows {
            events.push(row?);
        }
        Ok(events)
    }

    pub fn event_status(&self, id: i64) -> Result<DeliveryStatus> {
        let status: String = self.conn.query_row(
            "SELECT status FROM outbox_events WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        Ok(DeliveryStatus::from_str(&status))
    }

    pub fn mark_delivered(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE outbox_events
             SET status = ?1, delivered_at = datetime('now'), last_error = NULL
             WHERE id = ?2",
            params![DeliveryStatus::Delivered.as_str(), id],
        )?;
        Ok(())
    }

    pub fn mark_delivery_failed(&self, id: i64, error: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE outbox_events
             SET attempts = attempts + 1, last_attempt_at = datetime('now'), last_error = ?1
             WHERE id = ?2",
            params![error, id],
        )?;
        Ok(())
    }

    pub fn feed_cache(&self, url: &str) -> Result<Option<FeedCache>> {
        self.conn
            .query_row(
                "SELECT etag, last_modified FROM feed_cache WHERE url = ?1",
                params![url],
                |row| {
                    Ok(FeedCache {
                        etag: row.get(0)?,
                        last_modified: row.get(1)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn update_feed_cache(
        &self,
        url: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO feed_cache (url, etag, last_modified, updated_at)
             VALUES (?1, ?2, ?3, datetime('now'))
             ON CONFLICT(url) DO UPDATE SET
               etag = excluded.etag,
               last_modified = excluded.last_modified,
               updated_at = excluded.updated_at",
            params![url, etag, last_modified],
        )?;
        Ok(())
    }

    fn init(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS seen_urls (
                url TEXT PRIMARY KEY,
                first_seen_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS outbox_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                item_url TEXT NOT NULL UNIQUE,
                source_name TEXT NOT NULL,
                title TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                matched_rule TEXT NOT NULL,
                matched_entity TEXT,
                status TEXT NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                last_attempt_at TEXT,
                delivered_at TEXT,
                last_error TEXT
            );
            CREATE TABLE IF NOT EXISTS feed_cache (
                url TEXT PRIMARY KEY,
                etag TEXT,
                last_modified TEXT,
                updated_at TEXT NOT NULL
            );
            ",
        )?;
        Ok(())
    }
}
