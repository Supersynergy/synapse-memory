use crate::types::{DocId, MediaAsset, MediaKind};
use anyhow::Result;
use rusqlite::{Connection, params};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// Thin SQLite-backed media asset store.
/// Does NOT embed Synapse-core's Store — it writes to a separate SQLite file
/// so it can be used standalone. Hybrid search defers to synapse-core via text index.
pub struct MediaDb {
    conn: Connection,
}

impl MediaDb {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        migrate(&conn)?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        migrate(&conn)?;
        Ok(Self { conn })
    }

    /// Store a pre-built asset record.
    pub fn insert(&self, asset: &NewAsset) -> Result<DocId> {
        let meta = serde_json::to_string(&asset.metadata)?;
        self.conn.execute(
            "INSERT INTO media_assets (path, kind, mime, timestamp_sec, parent_id, caption, metadata)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                asset.path,
                asset.kind.to_string(),
                asset.mime,
                asset.timestamp,
                asset.parent_id,
                asset.caption,
                meta,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Full-text + kind filter search over captions/paths.
    pub fn search(&self, query: &str, filter: Option<MediaKind>) -> Result<Vec<MediaAsset>> {
        let q = format!("%{query}%");
        if let Some(kind) = filter {
            let mut stmt = self.conn.prepare(
                "SELECT id, path, kind, mime, timestamp_sec, parent_id, metadata
                 FROM media_assets
                 WHERE kind = ?1 AND (caption LIKE ?2 OR path LIKE ?2)
                 LIMIT 50",
            )?;
            let rows = stmt.query(params![kind.to_string(), q])?;
            collect_rows(rows)
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT id, path, kind, mime, timestamp_sec, parent_id, metadata
                 FROM media_assets
                 WHERE caption LIKE ?1 OR path LIKE ?1
                 LIMIT 50",
            )?;
            let rows = stmt.query(params![q])?;
            collect_rows(rows)
        }
    }

    pub fn get(&self, id: DocId) -> Result<Option<MediaAsset>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, kind, mime, timestamp_sec, parent_id, metadata
             FROM media_assets WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_asset(row)?))
        } else {
            Ok(None)
        }
    }

    /// List frames belonging to a parent video.
    pub fn frames_of(&self, parent_id: DocId) -> Result<Vec<MediaAsset>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, kind, mime, timestamp_sec, parent_id, metadata
             FROM media_assets WHERE parent_id = ?1 AND kind = 'frame'
             ORDER BY timestamp_sec ASC",
        )?;
        let rows = stmt.query(params![parent_id])?;
        collect_rows(rows)
    }
}

fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS media_assets (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            path          TEXT    NOT NULL,
            kind          TEXT    NOT NULL,
            mime          TEXT    NOT NULL DEFAULT '',
            timestamp_sec REAL,
            parent_id     INTEGER,
            caption       TEXT,
            metadata      TEXT    NOT NULL DEFAULT '{}'
         );
         CREATE INDEX IF NOT EXISTS idx_media_kind   ON media_assets(kind);
         CREATE INDEX IF NOT EXISTS idx_media_parent ON media_assets(parent_id);
         CREATE VIRTUAL TABLE IF NOT EXISTS media_fts USING fts5(
             caption, path, content='media_assets', content_rowid='id'
         );",
    )?;
    Ok(())
}

pub struct NewAsset {
    pub path: String,
    pub kind: MediaKind,
    pub mime: String,
    pub timestamp: Option<f32>,
    pub parent_id: Option<DocId>,
    pub caption: Option<String>,
    pub metadata: HashMap<String, Value>,
}

fn collect_rows(mut rows: rusqlite::Rows<'_>) -> Result<Vec<MediaAsset>> {
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(row_to_asset(row)?);
    }
    Ok(out)
}

fn row_to_asset(row: &rusqlite::Row<'_>) -> rusqlite::Result<MediaAsset> {
    let kind_str: String = row.get(2)?;
    let meta_str: String = row.get(6)?;
    let kind = kind_str.parse().unwrap_or(MediaKind::Image);
    let metadata: HashMap<String, Value> = serde_json::from_str(&meta_str).unwrap_or_default();
    Ok(MediaAsset {
        id: row.get(0)?,
        path: row.get(1)?,
        kind,
        mime: row.get(3)?,
        timestamp: row.get(4)?,
        parent_asset: row.get(5)?,
        metadata,
    })
}
