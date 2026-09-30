//! Event ingest for Synapse Ultra.
//!
//! Events are the beads-style log: every agent action (tool call, decision,
//! file write, message) becomes a row in `synapse_events`. BLAKE3 dedup
//! collapses identical content within the same session.

use crate::UltraResult;
use chrono::Utc;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

/// Event kinds. Extensible — unknown kinds are stored as-is (TEXT column).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    SessionStart,
    SessionEnd,
    ToolCall,
    ToolResult,
    Decision,
    FileWrite,
    FileRead,
    Message,
    Error,
    Feedback,
    Custom(String),
}

impl EventKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::SessionStart => "session_start",
            Self::SessionEnd => "session_end",
            Self::ToolCall => "tool_call",
            Self::ToolResult => "tool_result",
            Self::Decision => "decision",
            Self::FileWrite => "file_write",
            Self::FileRead => "file_read",
            Self::Message => "message",
            Self::Error => "error",
            Self::Feedback => "feedback",
            Self::Custom(s) => s.as_str(),
        }
    }
}

impl std::str::FromStr for EventKind {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "session_start" => Self::SessionStart,
            "session_end" => Self::SessionEnd,
            "tool_call" => Self::ToolCall,
            "tool_result" => Self::ToolResult,
            "decision" => Self::Decision,
            "file_write" => Self::FileWrite,
            "file_read" => Self::FileRead,
            "message" => Self::Message,
            "error" => Self::Error,
            "feedback" => Self::Feedback,
            other => Self::Custom(other.to_string()),
        })
    }
}

/// A single event to ingest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    #[serde(default = "default_ts")]
    pub ts: i64,
    pub session_id: Option<String>,
    pub agent: String,
    /// Event kind as a string (e.g. "decision", "tool_call", "message").
    /// Any string is accepted; known kinds map to [`EventKind`] variants.
    pub kind: String,
    pub uri: Option<String>,
    pub content: Option<String>,
    pub meta: Option<serde_json::Value>,
}

fn default_ts() -> i64 {
    Utc::now().timestamp()
}

impl Event {
    /// Create a new event with the current timestamp.
    pub fn now(agent: &str, kind: EventKind) -> Self {
        Self {
            ts: Utc::now().timestamp(),
            session_id: None,
            agent: agent.to_string(),
            kind: kind.as_str().to_string(),
            uri: None,
            content: None,
            meta: None,
        }
    }

    /// Compute the BLAKE3 dedup key: hash of (session_id, agent, kind, uri, content).
    pub fn dedup_key(&self) -> [u8; 32] {
        let mut input = Vec::with_capacity(128);
        if let Some(s) = &self.session_id {
            input.extend_from_slice(s.as_bytes());
        }
        input.push(0x1f);
        input.extend_from_slice(self.agent.as_bytes());
        input.push(0x1f);
        input.extend_from_slice(self.kind.as_bytes());
        input.push(0x1f);
        if let Some(u) = &self.uri {
            input.extend_from_slice(u.as_bytes());
        }
        input.push(0x1f);
        if let Some(c) = &self.content {
            input.extend_from_slice(c.as_bytes());
        }
        blake3::hash(&input).into()
    }

    /// Serialize to JSON string (for ingest_event_json).
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }
}

/// Filter for querying events.
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    pub agent: Option<String>,
    pub session_id: Option<String>,
    pub kind: Option<String>,
    pub uri: Option<String>,
    pub since: Option<i64>,
    pub until: Option<i64>,
    pub limit: Option<i64>,
}

impl EventFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn agent(mut self, a: impl Into<String>) -> Self {
        self.agent = Some(a.into());
        self
    }

    pub fn session(mut self, s: impl Into<String>) -> Self {
        self.session_id = Some(s.into());
        self
    }

    pub fn kind(mut self, k: impl Into<String>) -> Self {
        self.kind = Some(k.into());
        self
    }

    pub fn since(mut self, ts: i64) -> Self {
        self.since = Some(ts);
        self
    }

    pub fn until(mut self, ts: i64) -> Self {
        self.until = Some(ts);
        self
    }

    pub fn limit(mut self, n: i64) -> Self {
        self.limit = Some(n);
        self
    }

    pub fn uri(mut self, u: impl Into<String>) -> Self {
        self.uri = Some(u.into());
        self
    }

    fn where_clause(&self) -> (String, Vec<rusqlite::types::Value>) {
        let mut clauses = Vec::new();
        let mut binds = Vec::new();
        if let Some(a) = &self.agent {
            clauses.push("agent = ?");
            binds.push(rusqlite::types::Value::Text(a.clone()));
        }
        if let Some(s) = &self.session_id {
            clauses.push("session_id = ?");
            binds.push(rusqlite::types::Value::Text(s.clone()));
        }
        if let Some(k) = &self.kind {
            clauses.push("kind = ?");
            binds.push(rusqlite::types::Value::Text(k.clone()));
        }
        if let Some(u) = &self.uri {
            clauses.push("uri = ?");
            binds.push(rusqlite::types::Value::Text(u.clone()));
        }
        if let Some(ts) = self.since {
            clauses.push("ts >= ?");
            binds.push(rusqlite::types::Value::Integer(ts));
        }
        if let Some(ts) = self.until {
            clauses.push("ts <= ?");
            binds.push(rusqlite::types::Value::Integer(ts));
        }
        let sql = if clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", clauses.join(" AND "))
        };
        (sql, binds)
    }
}

/// Ingest a single event. Dedup by BLAKE3 key — if an event with the same
/// dedup key already exists, it is silently skipped.
pub fn ingest_event(conn: &Connection, event: &Event) -> UltraResult<i64> {
    let dedup = event.dedup_key();
    let kind_str = event.kind.as_str();
    let meta_json = event
        .meta
        .as_ref()
        .map(|v| serde_json::to_string(v).unwrap_or_default());

    // Check for existing event with same dedup key
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM synapse_events WHERE blake3 = ?1",
            params![dedup.as_slice()],
            |row| row.get(0),
        )
        .ok();
    if let Some(id) = existing {
        return Ok(id);
    }

    // Compress large content with zstd if feature is enabled and content > 1KB
    #[cfg(feature = "zstd-compress")]
    let (content_text, content_zst) = if let Some(c) = &event.content {
        if c.len() > 1024 {
            match zstd::encode_all(c.as_bytes(), 3) {
                Ok(blob) => (None, Some(blob)),
                Err(_) => (Some(c.clone()), None),
            }
        } else {
            (Some(c.clone()), None)
        }
    } else {
        (None, None)
    };
    #[cfg(not(feature = "zstd-compress"))]
    let (content_text, content_zst) = (event.content.clone(), None::<Vec<u8>>);

    // Auto-upsert session row so the FK on synapse_events.session_id is satisfied.
    if let Some(sid) = &event.session_id {
        conn.execute(
            "INSERT INTO sessions (session_id, agent, started_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(session_id) DO UPDATE SET ended_at = ?3",
            params![sid, event.agent, event.ts],
        )?;
    }

    conn.execute(
        "INSERT INTO synapse_events (ts, session_id, agent, kind, uri, content, content_zst, blake3, meta)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            event.ts,
            event.session_id,
            event.agent,
            kind_str,
            event.uri,
            content_text,
            content_zst,
            dedup.as_slice(),
            meta_json,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Input for [`ingest_decision`].
#[derive(Debug, Clone)]
pub struct DecisionInput<'a> {
    pub ts: i64,
    pub session_id: Option<&'a str>,
    pub agent: &'a str,
    pub uri: &'a str,
    pub rationale: Option<&'a str>,
    pub source_uri: Option<&'a str>,
    pub target_uri: Option<&'a str>,
    pub meta: Option<serde_json::Value>,
}

impl<'a> DecisionInput<'a> {
    /// Minimal constructor — optional fields default to `None`.
    pub fn new(ts: i64, agent: &'a str, uri: &'a str) -> Self {
        Self {
            ts,
            session_id: None,
            agent,
            uri,
            rationale: None,
            source_uri: None,
            target_uri: None,
            meta: None,
        }
    }
}

/// Ingest a decision. Also creates graph nodes + edges via trigger.
///
/// Dedup key includes rationale + source + target so that two decisions
/// on the same URI with different rationale are not collapsed.
pub fn ingest_decision(conn: &Connection, d: &DecisionInput<'_>) -> UltraResult<i64> {
    let DecisionInput {
        ts,
        session_id,
        agent,
        uri,
        rationale,
        source_uri,
        target_uri,
        meta,
    } = d;
    let mut input = Vec::new();
    input.extend_from_slice(uri.as_bytes());
    input.push(0x1f);
    input.extend_from_slice(agent.as_bytes());
    input.push(0x1f);
    if let Some(s) = session_id {
        input.extend_from_slice(s.as_bytes());
    }
    input.push(0x1f);
    if let Some(r) = rationale {
        input.extend_from_slice(r.as_bytes());
    }
    input.push(0x1f);
    if let Some(s) = source_uri {
        input.extend_from_slice(s.as_bytes());
    }
    input.push(0x1f);
    if let Some(t) = target_uri {
        input.extend_from_slice(t.as_bytes());
    }
    let dedup: [u8; 32] = blake3::hash(&input).into();
    let meta_json = meta
        .as_ref()
        .map(|v| serde_json::to_string(v).unwrap_or_default());

    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM decisions WHERE blake3 = ?1",
            params![dedup.as_slice()],
            |row| row.get(0),
        )
        .ok();
    if let Some(id) = existing {
        return Ok(id);
    }

    // Upsert session row so the FK on decisions.session_id is satisfied.
    if let Some(sid) = session_id {
        conn.execute(
            "INSERT INTO sessions (session_id, agent, started_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(session_id) DO UPDATE SET ended_at = ?3",
            params![sid, agent, ts],
        )?;
    }

    conn.execute(
        "INSERT INTO decisions (ts, session_id, agent, uri, rationale, source_uri, target_uri, blake3, meta)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            ts,
            session_id,
            agent,
            uri,
            rationale,
            source_uri,
            target_uri,
            dedup.as_slice(),
            meta_json,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Input for [`ingest_token_cost`].
#[derive(Debug, Clone)]
pub struct TokenCostInput<'a> {
    pub ts: i64,
    pub session_id: Option<&'a str>,
    pub agent: &'a str,
    pub model: &'a str,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub cost_usd: f64,
    pub meta: Option<serde_json::Value>,
}

/// Ingest a token cost record.
///
/// Upserts the session row so the FK on token_cost.session_id is satisfied.
pub fn ingest_token_cost(conn: &Connection, t: &TokenCostInput<'_>) -> UltraResult<i64> {
    let TokenCostInput {
        ts,
        session_id,
        agent,
        model,
        input_tokens,
        output_tokens,
        cache_read,
        cache_write,
        cost_usd,
        meta,
    } = t;
    let meta_json = meta
        .as_ref()
        .map(|v| serde_json::to_string(v).unwrap_or_default());
    if let Some(sid) = session_id {
        conn.execute(
            "INSERT INTO sessions (session_id, agent, started_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(session_id) DO UPDATE SET ended_at = ?3",
            params![sid, agent, ts],
        )?;
    }
    conn.execute(
        "INSERT INTO token_cost (ts, session_id, agent, model, input_tokens, output_tokens, cache_read, cache_write, cost_usd, meta)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            ts,
            session_id,
            agent,
            model,
            input_tokens,
            output_tokens,
            cache_read,
            cache_write,
            cost_usd,
            meta_json,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Query events matching a filter. Returns rows ordered by ts DESC.
pub fn query_events(conn: &Connection, filter: &EventFilter) -> UltraResult<Vec<EventRow>> {
    let (where_sql, binds) = filter.where_clause();
    let limit_sql = match filter.limit {
        Some(n) => format!(" LIMIT {}", n.max(1)),
        None => String::from(" LIMIT 1000"),
    };
    let sql = format!(
        "SELECT id, ts, session_id, agent, kind, uri, content, meta
         FROM synapse_events{where_sql}
         ORDER BY ts DESC{limit_sql}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(binds), |row| {
        let content: Option<String> = row.get(6)?;
        let meta: Option<String> = row.get(7)?;
        Ok(EventRow {
            id: row.get(0)?,
            ts: row.get(1)?,
            session_id: row.get(2)?,
            agent: row.get(3)?,
            kind: row.get(4)?,
            uri: row.get(5)?,
            content,
            meta,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// A row from the event log (as returned by query_events).
#[derive(Debug, Clone, Serialize)]
pub struct EventRow {
    pub id: i64,
    pub ts: i64,
    pub session_id: Option<String>,
    pub agent: String,
    pub kind: String,
    pub uri: Option<String>,
    pub content: Option<String>,
    pub meta: Option<String>,
}

/// Ingest an event from a JSON string. Accepts the `Event` shape.
pub fn ingest_event_json(conn: &Connection, json: &str) -> UltraResult<i64> {
    let event: Event =
        serde_json::from_str(json).map_err(|e| crate::UltraError::EventParse(e.to_string()))?;
    ingest_event(conn, &event)
}

/// Ingest a batch of events in a single transaction. Dedup by BLAKE3 key.
/// Returns the number of newly inserted events (duplicates are skipped).
///
/// Uses a single multi-VALUES INSERT prepared once and rebound per row —
/// 2-3x faster than per-row execute on large batches (one VDBE program
/// instead of N). Does its own dedup check inline so it can distinguish
/// new inserts from dedup hits.
pub fn ingest_events(conn: &Connection, events: &[Event]) -> UltraResult<usize> {
    if events.is_empty() {
        return Ok(0);
    }
    let mut inserted = 0usize;
    conn.execute_batch("BEGIN")?;

    // Pre-prepare the multi-VALUES INSERT (9 placeholders × 1 row).
    // We rebind + execute per event — one VDBE program, N executions.
    let insert_sql = "INSERT INTO synapse_events
        (ts, session_id, agent, kind, uri, content, content_zst, blake3, meta)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)";
    let session_sql = "INSERT INTO sessions (session_id, agent, started_at)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(session_id) DO UPDATE SET ended_at = ?3";
    let dedup_sql = "SELECT id FROM synapse_events WHERE blake3 = ?1";

    let mut insert_stmt = conn.prepare(insert_sql)?;
    let mut session_stmt = conn.prepare(session_sql)?;
    let mut dedup_stmt = conn.prepare(dedup_sql)?;

    for ev in events {
        let dedup = ev.dedup_key();
        let kind_str = ev.kind.as_str();
        let meta_json = ev
            .meta
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap_or_default());

        let existing: Option<i64> = dedup_stmt
            .query_row(params![dedup.as_slice()], |row| row.get(0))
            .ok();
        if existing.is_some() {
            continue;
        }

        #[cfg(feature = "zstd-compress")]
        let (content_text, content_zst) = if let Some(c) = &ev.content {
            if c.len() > 1024 {
                match zstd::encode_all(c.as_bytes(), 3) {
                    Ok(blob) => (None, Some(blob)),
                    Err(_) => (Some(c.clone()), None),
                }
            } else {
                (Some(c.clone()), None)
            }
        } else {
            (None, None)
        };
        #[cfg(not(feature = "zstd-compress"))]
        let (content_text, content_zst) = (ev.content.clone(), None::<Vec<u8>>);

        if let Some(sid) = &ev.session_id
            && let Err(e) = session_stmt.execute(params![sid, ev.agent, ev.ts])
        {
            conn.execute_batch("ROLLBACK")?;
            return Err(e.into());
        }

        if let Err(e) = insert_stmt.execute(params![
            ev.ts,
            ev.session_id,
            ev.agent,
            kind_str,
            ev.uri,
            content_text,
            content_zst,
            dedup.as_slice(),
            meta_json,
        ]) {
            conn.execute_batch("ROLLBACK")?;
            return Err(e.into());
        }
        inserted += 1;
    }
    conn.execute_batch("COMMIT")?;
    Ok(inserted)
}

/// Full-text search over `synapse_events.content` via the FTS5 index
/// (`synapse_events_fts`). Returns rows ordered by relevance (bm25()).
///
/// Query syntax: FTS5 standard — `unquoted terms`, `"exact phrase"`, `OR`,
/// `*` prefix, `column:term`. `LIMIT` defaults to 50, max 1000.
pub fn search_events(
    conn: &Connection,
    query: &str,
    limit: Option<i64>,
) -> UltraResult<Vec<EventRow>> {
    let n = limit.unwrap_or(50).clamp(1, 1000);
    let sql = "SELECT e.id, e.ts, e.session_id, e.agent, e.kind, e.uri, e.content, e.meta
        FROM synapse_events_fts f
        JOIN synapse_events e ON e.id = f.rowid
        WHERE synapse_events_fts MATCH ?1
        ORDER BY bm25(synapse_events_fts) ASC
        LIMIT ?2";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![query, n], |row| {
        Ok(EventRow {
            id: row.get(0)?,
            ts: row.get(1)?,
            session_id: row.get(2)?,
            agent: row.get(3)?,
            kind: row.get(4)?,
            uri: row.get(5)?,
            content: row.get(6)?,
            meta: row.get(7)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Ingest a batch of events from a JSONL file (one JSON event per line).
/// Skips blank lines and lines that fail to parse (with a warning to stderr).
/// Runs the whole batch in a single transaction for throughput.
pub fn ingest_jsonl_file(conn: &Connection, path: &std::path::Path) -> UltraResult<usize> {
    let content = std::fs::read_to_string(path)?;
    let mut count = 0usize;
    conn.execute_batch("BEGIN")?;
    for (lineno, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match ingest_event_json(conn, line) {
            Ok(_) => count += 1,
            Err(e) => {
                conn.execute_batch("ROLLBACK")?;
                eprintln!("ultra: ingest abort at line {}: {e}", lineno + 1);
                return Err(e);
            }
        }
    }
    conn.execute_batch("COMMIT")?;
    Ok(count)
}
