//! # synapse-audit
//!
//! Append-only hash-chained audit trail for synapse-memory. Every Read/Write/Delete
//! is recorded as an event whose hash chains to the previous event (blake3).
//! Tampering with any event breaks the chain. Stored in a separate `audit.db`
//! (WAL mode) so it can be rotated/backed up independently.
//!
//! ## Schema
//!
//! ```sql
//! CREATE TABLE audit_events (
//!   id INTEGER PRIMARY KEY AUTOINCREMENT,
//!   ts INTEGER NOT NULL,
//!   actor TEXT NOT NULL,
//!   action TEXT NOT NULL,   -- 'read' | 'write' | 'delete' | 'grant' | 'revoke'
//!   target TEXT NOT NULL,   -- doc id / space / user
//!   space TEXT,
//!   meta TEXT,              -- JSON blob with context
//!   prev_hash BLOB,         -- 32 bytes
//!   hash BLOB NOT NULL      -- 32 bytes = blake3(prev_hash || event)
//! );
//! ```

use anyhow::Result;
use blake3::Hasher;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuditError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("chain broken at id={id}: expected {expected:?}, got {got:?}")]
    BrokenChain {
        id: i64,
        expected: [u8; 32],
        got: [u8; 32],
    },
    #[error("empty chain")]
    Empty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Read,
    Write,
    Delete,
    Grant,
    Revoke,
}

impl Action {
    pub fn as_str(&self) -> &'static str {
        match self {
            Action::Read => "read",
            Action::Write => "write",
            Action::Delete => "delete",
            Action::Grant => "grant",
            Action::Revoke => "revoke",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "read" => Some(Action::Read),
            "write" => Some(Action::Write),
            "delete" => Some(Action::Delete),
            "grant" => Some(Action::Grant),
            "revoke" => Some(Action::Revoke),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: i64,
    pub ts: i64,
    pub actor: String,
    pub action: Action,
    pub target: String,
    pub space: Option<String>,
    pub meta: Option<String>,
    pub prev_hash: Option<[u8; 32]>,
    pub hash: [u8; 32],
}

/// Initialize audit schema.
pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS audit_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts INTEGER NOT NULL,
            actor TEXT NOT NULL,
            action TEXT NOT NULL,
            target TEXT NOT NULL,
            space TEXT,
            meta TEXT,
            prev_hash BLOB,
            hash BLOB NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_audit_ts ON audit_events(ts);
        CREATE INDEX IF NOT EXISTS idx_audit_actor ON audit_events(actor);
        CREATE INDEX IF NOT EXISTS idx_audit_space ON audit_events(space);",
    )?;
    Ok(())
}

/// Compute the hash of an event given its fields and prev_hash.
fn event_hash(
    ts: i64,
    actor: &str,
    action: &str,
    target: &str,
    space: Option<&str>,
    meta: Option<&str>,
    prev_hash: Option<&[u8; 32]>,
) -> [u8; 32] {
    let mut h = Hasher::new();
    if let Some(p) = prev_hash {
        h.update(p);
    } else {
        h.update(&[0u8; 32]);
    }
    h.update(&ts.to_le_bytes());
    h.update(actor.as_bytes());
    h.update(action.as_bytes());
    h.update(target.as_bytes());
    if let Some(s) = space {
        h.update(s.as_bytes());
    }
    if let Some(m) = meta {
        h.update(m.as_bytes());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(h.finalize().as_bytes());
    out
}

/// Get the hash of the last event in the chain (or None if empty).
pub fn last_hash(conn: &Connection) -> Result<Option<[u8; 32]>> {
    conn.query_row(
        "SELECT hash FROM audit_events ORDER BY id DESC LIMIT 1",
        [],
        |r| {
            let blob: Vec<u8> = r.get(0)?;
            let mut h = [0u8; 32];
            h.copy_from_slice(&blob);
            Ok(h)
        },
    )
    .optional()
    .map_err(Into::into)
}

/// Append an event to the chain. Returns the inserted id.
pub fn append(
    conn: &Connection,
    actor: &str,
    action: Action,
    target: &str,
    space: Option<&str>,
    meta: Option<&str>,
) -> Result<i64> {
    let ts = Utc::now().timestamp();
    let prev = last_hash(conn)?;
    let hash = event_hash(
        ts,
        actor,
        action.as_str(),
        target,
        space,
        meta,
        prev.as_ref(),
    );
    conn.execute(
        "INSERT INTO audit_events (ts, actor, action, target, space, meta, prev_hash, hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            ts,
            actor,
            action.as_str(),
            target,
            space,
            meta,
            prev.as_ref().map(|h| h.to_vec()),
            hash.to_vec(),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Query events in a time range (ts >= since AND ts < until). Returns ascending.
pub fn query_range(conn: &Connection, since: i64, until: i64) -> Result<Vec<AuditEvent>> {
    let mut stmt = conn.prepare(
        "SELECT id, ts, actor, action, target, space, meta, prev_hash, hash
         FROM audit_events
         WHERE ts >= ?1 AND ts < ?2
         ORDER BY id ASC",
    )?;
    let rows = stmt.query_map(params![since, until], |r| {
        let action_s: String = r.get(3)?;
        let space: Option<String> = r.get(5)?;
        let meta: Option<String> = r.get(6)?;
        let prev_blob: Option<Vec<u8>> = r.get(7)?;
        let hash_blob: Vec<u8> = r.get(8)?;
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&hash_blob);
        let prev_hash = prev_blob.map(|b| {
            let mut h = [0u8; 32];
            h.copy_from_slice(&b);
            h
        });
        Ok(AuditEvent {
            id: r.get(0)?,
            ts: r.get(1)?,
            actor: r.get(2)?,
            action: Action::from_str(&action_s).unwrap_or(Action::Read),
            target: r.get(4)?,
            space,
            meta,
            prev_hash,
            hash,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Verify the hash chain from start to end. Returns Err on first broken link.
pub fn verify_chain(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id, ts, actor, action, target, space, meta, prev_hash, hash
         FROM audit_events ORDER BY id ASC",
    )?;
    let mut rows = stmt.query([])?;
    let mut prev: Option<[u8; 32]> = None;
    while let Some(r) = rows.next()? {
        let id: i64 = r.get(0)?;
        let ts: i64 = r.get(1)?;
        let actor: String = r.get(2)?;
        let action: String = r.get(3)?;
        let target: String = r.get(4)?;
        let space: Option<String> = r.get(5)?;
        let meta: Option<String> = r.get(6)?;
        let prev_blob: Option<Vec<u8>> = r.get(7)?;
        let hash_blob: Vec<u8> = r.get(8)?;
        let mut got = [0u8; 32];
        got.copy_from_slice(&hash_blob);
        let expected = event_hash(
            ts,
            &actor,
            &action,
            &target,
            space.as_deref(),
            meta.as_deref(),
            prev.as_ref(),
        );
        if expected != got {
            return Err(AuditError::BrokenChain {
                id,
                expected,
                got,
            }
            .into());
        }
        let stored_prev = prev_blob.map(|b| {
            let mut h = [0u8; 32];
            h.copy_from_slice(&b);
            h
        });
        if stored_prev != prev {
            return Err(AuditError::BrokenChain {
                id,
                expected: prev.unwrap_or([0u8; 32]),
                got: stored_prev.unwrap_or([0u8; 32]),
            }
            .into());
        }
        prev = Some(got);
    }
    Ok(())
}

/// Count events.
pub fn count(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn fresh_conn() -> Connection {
        let f = NamedTempFile::new().unwrap().keep().unwrap().1;
        let conn = Connection::open(&f).unwrap();
        init_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn append_chains_to_prev() {
        let conn = fresh_conn();
        let id1 = append(&conn, "alice", Action::Write, "doc1", Some("acme"), None).unwrap();
        let id2 = append(&conn, "bob", Action::Read, "doc1", Some("acme"), None).unwrap();
        assert!(id2 > id1);
        assert_eq!(count(&conn).unwrap(), 2);
    }

    #[test]
    fn verify_chain_on_clean_db() {
        let conn = fresh_conn();
        append(&conn, "alice", Action::Write, "doc1", Some("acme"), None).unwrap();
        append(&conn, "bob", Action::Read, "doc1", Some("acme"), None).unwrap();
        append(&conn, "alice", Action::Delete, "doc2", None, None).unwrap();
        verify_chain(&conn).unwrap();
    }

    #[test]
    fn tamper_detected() {
        let conn = fresh_conn();
        append(&conn, "alice", Action::Write, "doc1", Some("acme"), None).unwrap();
        append(&conn, "bob", Action::Read, "doc1", Some("acme"), None).unwrap();
        // Tamper: change actor of first event.
        conn.execute(
            "UPDATE audit_events SET actor = 'eve' WHERE id = 1",
            [],
        )
        .unwrap();
        let r = verify_chain(&conn);
        assert!(matches!(r, Err(e) if e.to_string().contains("broken")));
    }

    #[test]
    fn query_range_filters_by_time() {
        let conn = fresh_conn();
        let t0 = Utc::now().timestamp();
        append(&conn, "alice", Action::Write, "doc1", Some("acme"), None).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        append(&conn, "bob", Action::Read, "doc1", Some("acme"), None).unwrap();
        let later = Utc::now().timestamp() + 1;
        let events = query_range(&conn, t0, later).unwrap();
        assert_eq!(events.len(), 2);
        let only_first = query_range(&conn, t0, t0 + 1).unwrap();
        assert!(only_first.len() <= 1);
    }

    #[test]
    fn empty_chain_verifies() {
        let conn = fresh_conn();
        verify_chain(&conn).unwrap();
    }

    #[test]
    fn meta_is_hashed() {
        let conn = fresh_conn();
        append(&conn, "alice", Action::Write, "doc1", Some("acme"), Some("{\"k\":1}")).unwrap();
        // Tamper meta
        conn.execute(
            "UPDATE audit_events SET meta = '{\"k\":2}' WHERE id = 1",
            [],
        )
        .unwrap();
        let r = verify_chain(&conn);
        assert!(r.is_err(), "meta change must break chain");
    }

    #[test]
    fn action_roundtrip() {
        for a in [Action::Read, Action::Write, Action::Delete, Action::Grant, Action::Revoke] {
            assert_eq!(Action::from_str(a.as_str()), Some(a));
        }
    }
}
