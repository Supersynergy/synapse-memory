//! MCP tool surface for synapse-space.
//!
//! 11 tools implemented as JSON-in/JSON-out functions.
//! Wire into synapse-mcp as a registrable subtree (see TODO below).
//!
//! TODO: remaining 18 tools from full Spaces MCP spec.
//! TODO: register via synapse-mcp's tool registry instead of standalone.

use serde::Deserialize;
use serde_json::{json, Value};
use synapse_core::types::PutRequest;
extern crate blake3;

// ---------------------------------------------------------------------------
// Tool input/output shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SpaceCreateInput {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct WingAddInput {
    pub space_path: String,
    pub wing_name: String,
}

#[derive(Debug, Deserialize)]
pub struct RoomAddInput {
    pub space_path: String,
    pub wing_name: String,
    pub room_topic: String,
}

#[derive(Debug, Deserialize)]
pub struct DrawerPutInput {
    pub space_path: String,
    pub wing_name: String,
    pub room_topic: String,
    pub text: String,
    /// Optional pre-computed embedding (f32 list). If omitted, FTS-only storage.
    pub embedding: Option<Vec<f32>>,
}

#[derive(Debug, Deserialize)]
pub struct SpaceSearchInput {
    pub space_path: String,
    pub query: String,
    pub limit: Option<usize>,
    /// Optional pre-computed query embedding for hybrid search.
    pub embedding: Option<Vec<f32>>,
}

#[derive(Debug, Deserialize)]
pub struct SpaceWakeUpInput {
    pub space_path: String,
}

// P0 tools — new inputs

#[derive(Debug, Deserialize)]
pub struct DrawerListInput {
    pub space_path: String,
    pub wing: String,
    pub room: String,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct DrawerShowInput {
    pub space_path: String,
    pub id: i64,
}

#[derive(Debug, Deserialize)]
pub struct DrawerDeleteInput {
    pub space_path: String,
    pub id: i64,
}

#[derive(Debug, Deserialize)]
pub struct SweepMessage {
    pub role: String,
    pub content: String,
    pub ts: String,
}

/// New P0 input: raw session blob OR structured messages.
/// Exactly one of `session_text` or `messages` must be supplied.
#[derive(Debug, Deserialize)]
pub struct SpaceSweepInput {
    pub space_path: String,
    pub wing: String,
    pub room: String,
    /// Raw conversation blob (may be JSON array of messages or plain text).
    /// When supplied, the sweep function chunks automatically.
    pub session_text: Option<String>,
    /// Pre-split messages (legacy / test path).
    pub messages: Option<Vec<SweepMessage>>,
    /// Source session id for metadata (optional).
    pub source_session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WingSearchInput {
    pub space_path: String,
    pub wing: String,
    pub query: String,
    pub k: Option<usize>,
}

// ---------------------------------------------------------------------------
// Tool dispatch
// ---------------------------------------------------------------------------

/// Dispatch a named MCP tool call. Returns JSON result or JSON error.
pub fn dispatch(tool: &str, input: Value) -> Value {
    match tool {
        "space_create"  => space_create(input),
        "wing_add"      => wing_add(input),
        "room_add"      => room_add(input),
        "drawer_put"    => drawer_put(input),
        "space_search"  => space_search(input),
        "space_wake_up" => space_wake_up(input),
        "drawer_list"   => drawer_list(input),
        "drawer_show"   => drawer_show(input),
        "drawer_delete" => drawer_delete(input),
        "space_sweep"   => space_sweep(input),
        "wing_search"   => wing_search(input),
        other => json!({ "error": format!("unknown tool: {other}") }),
    }
}

// ---------------------------------------------------------------------------
// Original 6 tools
// ---------------------------------------------------------------------------

fn space_create(input: Value) -> Value {
    let Ok(inp) = serde_json::from_value::<SpaceCreateInput>(input) else {
        return json!({ "error": "invalid input" });
    };
    match crate::Space::open(&inp.name, &inp.path) {
        Ok(_) => json!({ "ok": true, "name": inp.name, "path": inp.path }),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

fn wing_add(input: Value) -> Value {
    let Ok(inp) = serde_json::from_value::<WingAddInput>(input) else {
        return json!({ "error": "invalid input" });
    };
    match crate::Space::open(&inp.wing_name, &inp.space_path) {
        Ok(mut s) => {
            let _ = s.wing(&inp.wing_name);
            json!({ "ok": true, "wing": inp.wing_name })
        }
        Err(e) => json!({ "error": e.to_string() }),
    }
}

fn room_add(input: Value) -> Value {
    let Ok(inp) = serde_json::from_value::<RoomAddInput>(input) else {
        return json!({ "error": "invalid input" });
    };
    match crate::Space::open(&inp.wing_name, &inp.space_path) {
        Ok(mut s) => {
            let w = s.wing(&inp.wing_name);
            let _ = w.room(&inp.room_topic);
            json!({ "ok": true, "wing": inp.wing_name, "room": inp.room_topic })
        }
        Err(e) => json!({ "error": e.to_string() }),
    }
}

fn drawer_put(input: Value) -> Value {
    let Ok(inp) = serde_json::from_value::<DrawerPutInput>(input) else {
        return json!({ "error": "invalid input" });
    };
    match crate::Space::open(&inp.wing_name, &inp.space_path) {
        Ok(mut s) => {
            let w = s.wing(&inp.wing_name);
            let mut r = w.room(&inp.room_topic);
            match r.put(inp.text, inp.embedding) {
                Ok(drawer) => json!({ "ok": true, "id": drawer.id, "uri": drawer.uri }),
                Err(e) => json!({ "error": e.to_string() }),
            }
        }
        Err(e) => json!({ "error": e.to_string() }),
    }
}

fn space_search(input: Value) -> Value {
    let Ok(inp) = serde_json::from_value::<SpaceSearchInput>(input) else {
        return json!({ "error": "invalid input" });
    };
    let limit = inp.limit.unwrap_or(10);
    match crate::Space::open("search", &inp.space_path) {
        Ok(s) => {
            let emb_ref = inp.embedding.as_deref();
            match s.search(&inp.query, emb_ref, limit) {
                Ok(hits) => {
                    let results: Vec<Value> = hits.iter().map(|h| json!({
                        "id": h.id,
                        "text": h.text,
                        "score": h.score,
                    })).collect();
                    json!({ "ok": true, "results": results })
                }
                Err(e) => json!({ "error": e.to_string() }),
            }
        }
        Err(e) => json!({ "error": e.to_string() }),
    }
}

fn space_wake_up(input: Value) -> Value {
    let Ok(inp) = serde_json::from_value::<SpaceWakeUpInput>(input) else {
        return json!({ "error": "invalid input" });
    };
    match crate::Space::open("wake", &inp.space_path) {
        Ok(s) => json!({ "ok": true, "space": s.name(), "status": "awake" }),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

// ---------------------------------------------------------------------------
// P0 tools — drawer_list, drawer_show, drawer_delete, space_sweep, wing_search
// ---------------------------------------------------------------------------

/// `drawer_list`: list drawers in a wing/room, sorted by recency desc.
/// URI pattern: `spaces://<wing>/<room>/%`
fn drawer_list(input: Value) -> Value {
    let Ok(inp) = serde_json::from_value::<DrawerListInput>(input) else {
        return json!({ "error": "invalid input" });
    };
    let limit = inp.limit.unwrap_or(50);
    let offset = inp.offset.unwrap_or(0);
    match crate::Space::open("list", &inp.space_path) {
        Ok(s) => {
            let prefix = format!("spaces://{}/{}/", inp.wing, inp.room);
            let sql = "SELECT id, uri, title, text, ts FROM docs \
                       WHERE uri LIKE ?1 AND (meta IS NULL OR meta NOT LIKE '%\"deleted\":true%') \
                       ORDER BY ts DESC LIMIT ?2 OFFSET ?3";
            let pattern = format!("{prefix}%");
            let rows: Result<Vec<_>, _> = s.conn_ref().prepare(sql)
                .and_then(|mut stmt| {
                    stmt.query_map(
                        rusqlite::params![pattern, limit as i64, offset as i64],
                        |row| Ok(json!({
                            "id":    row.get::<_, i64>(0)?,
                            "uri":   row.get::<_, String>(1)?,
                            "title": row.get::<_, Option<String>>(2)?,
                            "text":  row.get::<_, String>(3)?,
                            "ts":    row.get::<_, i64>(4)?,
                        })),
                    ).and_then(|it| it.collect())
                });
            match rows {
                Ok(r) => json!({ "ok": true, "drawers": r }),
                Err(e) => json!({ "error": e.to_string() }),
            }
        }
        Err(e) => json!({ "error": e.to_string() }),
    }
}

/// `drawer_show`: fetch one drawer by id, full content + metadata.
fn drawer_show(input: Value) -> Value {
    let Ok(inp) = serde_json::from_value::<DrawerShowInput>(input) else {
        return json!({ "error": "invalid input" });
    };
    match crate::Space::open("show", &inp.space_path) {
        Ok(s) => {
            let sql = "SELECT id, uri, title, text, meta, ts FROM docs WHERE id = ?1";
            let row = s.conn_ref().query_row(sql, rusqlite::params![inp.id], |row| {
                Ok(json!({
                    "id":    row.get::<_, i64>(0)?,
                    "uri":   row.get::<_, Option<String>>(1)?,
                    "title": row.get::<_, Option<String>>(2)?,
                    "text":  row.get::<_, String>(3)?,
                    "meta":  row.get::<_, Option<String>>(4)?,
                    "ts":    row.get::<_, i64>(5)?,
                }))
            });
            match row {
                Ok(r) => json!({ "ok": true, "drawer": r }),
                Err(rusqlite::Error::QueryReturnedNoRows) => json!({ "error": "not found" }),
                Err(e) => json!({ "error": e.to_string() }),
            }
        }
        Err(e) => json!({ "error": e.to_string() }),
    }
}

/// `drawer_delete`: soft-delete by id (stores `{"deleted":true}` in meta).
/// Physical row is kept; excluded from `drawer_list` and `space_search`.
fn drawer_delete(input: Value) -> Value {
    let Ok(inp) = serde_json::from_value::<DrawerDeleteInput>(input) else {
        return json!({ "error": "invalid input" });
    };
    match crate::Space::open("delete", &inp.space_path) {
        Ok(s) => {
            let sql = "UPDATE docs SET meta = json_patch(COALESCE(meta,'{}'), '{\"deleted\":true}') \
                       WHERE id = ?1";
            match s.conn_ref().execute(sql, rusqlite::params![inp.id]) {
                Ok(0) => json!({ "error": "not found" }),
                Ok(_) => json!({ "ok": true, "id": inp.id, "deleted": true }),
                Err(e) => json!({ "error": e.to_string() }),
            }
        }
        Err(e) => json!({ "error": e.to_string() }),
    }
}

/// `space_sweep`: chunk a session blob and insert each chunk as a Drawer.
///
/// Chunking strategy:
/// 1. If `session_text` looks like a JSON array of `{role, content}` objects →
///    parse as structured messages, one Drawer per message.
/// 2. Else if `messages` supplied → use directly (legacy path).
/// 3. Else → paragraph split (`\n\n`) with 512-token (≈2048 char) size target.
///
/// Idempotent: blake3 hash of `wing|room|chunk_text` used as URI suffix.
/// Skips if URI already exists in docs.
fn space_sweep(input: Value) -> Value {
    let Ok(inp) = serde_json::from_value::<SpaceSweepInput>(input) else {
        return json!({ "error": "invalid input: need space_path, wing, room, and session_text or messages" });
    };
    let source_id = inp.source_session_id.as_deref().unwrap_or("unknown");

    // --- produce chunks ---
    let chunks: Vec<(String, Value)> = if let Some(ref raw) = inp.session_text {
        chunk_session(raw, source_id)
    } else if let Some(ref msgs) = inp.messages {
        // Serialize messages to JSON array and route through chunk_session
        // so long messages get the same windowed splitting as session_text path.
        let arr: Vec<serde_json::Value> = msgs.iter().map(|m| {
            serde_json::json!({"role": m.role, "content": m.content, "ts": m.ts})
        }).collect();
        if let Ok(raw) = serde_json::to_string(&arr) {
            chunk_session(&raw, source_id)
        } else {
            return json!({ "error": "failed to serialize messages" });
        }
    } else {
        return json!({ "error": "supply session_text or messages" });
    };

    match crate::Space::open(&inp.wing, &inp.space_path) {
        Ok(mut s) => {
            let mut inserted = 0u32;
            let mut skipped = 0u32;
            for (idx, (chunk_text, meta)) in chunks.iter().enumerate() {
                let canonical = format!("{}|{}|{}", inp.wing, inp.room, chunk_text);
                let hash: [u8; 32] = *blake3::hash(canonical.as_bytes()).as_bytes();
                let uri = format!("spaces://{}/{}/sweep-{}", inp.wing, inp.room, &hex(&hash)[..16]);
                let exists: bool = s.conn_ref()
                    .query_row("SELECT 1 FROM docs WHERE uri = ?1",
                               rusqlite::params![&uri], |_| Ok(true))
                    .unwrap_or(false);
                if exists {
                    skipped += 1;
                    continue;
                }
                let req = PutRequest {
                    uri: Some(uri),
                    title: Some(format!("sweep chunk {} / {source_id}", idx)),
                    text: chunk_text.clone(),
                    meta: Some(meta.clone()),
                    embedding: None,
                };
                match s.store_put(&req) {
                    Ok(_) => inserted += 1,
                    Err(_) => skipped += 1,
                }
            }
            json!({ "ok": true, "inserted": inserted, "skipped": skipped, "total_chunks": chunks.len() })
        }
        Err(e) => json!({ "error": e.to_string() }),
    }
}

/// Parse/chunk a raw session blob into `(text, meta)` pairs.
///
/// JSON message array → ONE chunk per message. Long messages (>1000 chars) are
/// split into 400-char windows with 50-char overlap, preserving role prefix.
/// JSONL/paragraph fallback path is unchanged.
fn chunk_session(raw: &str, source_id: &str) -> Vec<(String, serde_json::Value)> {
    // Try JSON array of messages — one chunk per message
    if let Ok(msgs) = serde_json::from_str::<Vec<serde_json::Value>>(raw) {
        let mut parsed: Vec<(String, serde_json::Value)> = Vec::new();
        for (msg_idx, m) in msgs.iter().enumerate() {
            let Some(role) = m.get("role").and_then(|v| v.as_str()) else { continue };
            let Some(content) = m.get("content").and_then(|v| v.as_str()) else { continue };
            if content.trim().is_empty() { continue; }
            let ts = m.get("ts").or_else(|| m.get("timestamp"))
                .and_then(|v| v.as_str()).unwrap_or("");
            let prefix = if ts.is_empty() {
                format!("[{role}|msg{msg_idx}] ")
            } else {
                format!("[{role}|{ts}|msg{msg_idx}] ")
            };

            split_message_content(content, &prefix, role, ts, msg_idx, source_id, &mut parsed);
        }
        if !parsed.is_empty() {
            return parsed;
        }
    }

    // Try line-by-line JSON messages (JSONL or embedded within a larger blob)
    let mut json_line_chunks: Vec<(String, serde_json::Value)> = Vec::new();
    for (msg_idx, line) in raw.lines().enumerate() {
        if !line.trim_start().starts_with('{') { continue; }
        let Ok(m) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let Some(role) = m.get("role").and_then(|v| v.as_str()) else { continue };
        let Some(content) = m.get("content").and_then(|v| v.as_str()) else { continue };
        if content.trim().is_empty() { continue; }
        let prefix = format!("[{role}|msg{msg_idx}] ");
        split_message_content(content, &prefix, role, "", msg_idx, source_id, &mut json_line_chunks);
    }
    if !json_line_chunks.is_empty() {
        return json_line_chunks;
    }

    // Paragraph split — ~512 tokens = ~2048 chars target, hard cap 4096
    const TARGET: usize = 2048;
    const MAX: usize = 4096;
    let mut chunks = Vec::new();
    let mut buf = String::new();
    for para in raw.split("\n\n") {
        let para = para.trim();
        if para.is_empty() { continue; }
        if buf.len() + para.len() + 2 > MAX && !buf.is_empty() {
            let meta = serde_json::json!({"source": source_id, "chunk_type": "paragraph"});
            chunks.push((buf.trim().to_string(), meta));
            buf.clear();
        }
        if !buf.is_empty() { buf.push_str("\n\n"); }
        buf.push_str(para);
        if buf.len() >= TARGET {
            let meta = serde_json::json!({"source": source_id, "chunk_type": "paragraph"});
            chunks.push((buf.trim().to_string(), meta));
            buf.clear();
        }
    }
    if !buf.trim().is_empty() {
        let meta = serde_json::json!({"source": source_id, "chunk_type": "paragraph"});
        chunks.push((buf.trim().to_string(), meta));
    }
    chunks
}

/// Emit one or more `(text, meta)` chunks for a single message content.
/// If content <= 1000 chars: one chunk. Else: 400-char windows, 50-char overlap.
fn split_message_content(
    content: &str,
    prefix: &str,
    role: &str,
    ts: &str,
    msg_idx: usize,
    source_id: &str,
    out: &mut Vec<(String, serde_json::Value)>,
) {
    const WINDOW: usize = 400;
    const OVERLAP: usize = 50;
    const THRESHOLD: usize = 1000;

    let content = content.trim();
    if content.len() <= THRESHOLD {
        let text = format!("{prefix}{content}");
        let meta = serde_json::json!({"role": role, "ts": ts, "msg_idx": msg_idx, "source": source_id, "win": 0});
        out.push((text, meta));
        return;
    }

    // Windowed split on char boundaries
    let chars: Vec<char> = content.chars().collect();
    let total = chars.len();
    let step = WINDOW.saturating_sub(OVERLAP);
    let mut win_idx = 0usize;
    let mut start = 0usize;
    while start < total {
        let end = (start + WINDOW).min(total);
        let slice: String = chars[start..end].iter().collect();
        let text = format!("{prefix}{slice}");
        let meta = serde_json::json!({"role": role, "ts": ts, "msg_idx": msg_idx, "win": win_idx, "source": source_id});
        out.push((text, meta));
        win_idx += 1;
        if end == total { break; }
        start += step;
    }
}

/// `wing_search`: same as `space_search` but URI-filtered to one wing.
fn wing_search(input: Value) -> Value {
    let Ok(inp) = serde_json::from_value::<WingSearchInput>(input) else {
        return json!({ "error": "invalid input" });
    };
    let k = inp.k.unwrap_or(10);
    match crate::Space::open("wing_search", &inp.space_path) {
        Ok(s) => {
            let prefix = format!("spaces://{}/%", inp.wing);
            // FTS5 search then filter by URI prefix
            match s.search(&inp.query, None, k * 4) {
                Ok(all_hits) => {
                    let results: Vec<Value> = all_hits
                        .into_iter()
                        .filter(|h| {
                            // Re-fetch uri for this id to apply wing filter
                            s.conn_ref().query_row(
                                "SELECT uri FROM docs WHERE id = ?1 AND uri LIKE ?2",
                                rusqlite::params![h.id, prefix],
                                |_| Ok(()),
                            ).is_ok()
                        })
                        .take(k)
                        .map(|h| json!({ "id": h.id, "text": h.text, "score": h.score }))
                        .collect();
                    json!({ "ok": true, "wing": inp.wing, "results": results })
                }
                Err(e) => json!({ "error": e.to_string() }),
            }
        }
        Err(e) => json!({ "error": e.to_string() }),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
