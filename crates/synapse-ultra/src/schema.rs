//! Schema migration for Synapse Ultra.
//!
//! Additive-only: creates new tables alongside the existing synapse-memory
//! schema (docs / docs_fts / docs_vec / query_logs / meta). Never modifies
//! or drops existing tables. Idempotent — safe to run repeatedly.

use crate::UltraResult;
use rusqlite::Connection;

/// Current Synapse Ultra schema version. Bump when adding new tables/columns.
pub const ULTRA_SCHEMA_VERSION: u32 = 3;

/// Run the idempotent migration. Creates all Ultra tables, indexes, views,
/// and triggers. Safe to call on a fresh DB or an existing synapse-memory brain.db.
pub fn migrate(conn: &Connection) -> UltraResult<()> {
    // --- sessions: groups related events (e.g. one Claude Code session) ---
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS sessions (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id  TEXT UNIQUE NOT NULL,
    agent       TEXT NOT NULL,
    started_at  INTEGER NOT NULL,
    ended_at    INTEGER,
    meta        TEXT
);
CREATE INDEX IF NOT EXISTS idx_sessions_agent ON sessions(agent);
CREATE INDEX IF NOT EXISTS idx_sessions_started ON sessions(started_at);
"#,
    )?;

    // --- synapse_events: beads-style event log ---
    // content is stored as TEXT (or zstd-compressed BLOB if > threshold).
    // blake3 is the dedup key — identical content+uri+kind within the same
    // session is collapsed to one row.
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS synapse_events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    ts          INTEGER NOT NULL,
    session_id  TEXT,
    agent       TEXT NOT NULL,
    kind        TEXT NOT NULL,
    uri         TEXT,
    content     TEXT,
    content_zst BLOB,
    blake3      BLOB NOT NULL,
    meta        TEXT,
    FOREIGN KEY (session_id) REFERENCES sessions(session_id) ON DELETE SET NULL
);
CREATE INDEX IF NOT EXISTS idx_events_ts ON synapse_events(ts);
CREATE INDEX IF NOT EXISTS idx_events_agent_ts ON synapse_events(agent, ts);
CREATE INDEX IF NOT EXISTS idx_events_session_ts ON synapse_events(session_id, ts);
CREATE INDEX IF NOT EXISTS idx_events_kind_ts ON synapse_events(kind, ts);
CREATE INDEX IF NOT EXISTS idx_events_uri ON synapse_events(uri);
CREATE INDEX IF NOT EXISTS idx_events_blake3 ON synapse_events(blake3);
"#,
    )?;

    // --- decisions: a subset of events that represent agent decisions ---
    // A decision has a rationale and links to source/target URIs (graph edges).
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS decisions (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    ts          INTEGER NOT NULL,
    session_id  TEXT,
    agent       TEXT NOT NULL,
    uri         TEXT NOT NULL,
    rationale   TEXT,
    source_uri  TEXT,
    target_uri  TEXT,
    blake3      BLOB NOT NULL UNIQUE,
    meta        TEXT
);
CREATE INDEX IF NOT EXISTS idx_decisions_ts ON decisions(ts);
CREATE INDEX IF NOT EXISTS idx_decisions_agent ON decisions(agent);
CREATE INDEX IF NOT EXISTS idx_decisions_uri ON decisions(uri);
CREATE INDEX IF NOT EXISTS idx_decisions_session ON decisions(session_id);
"#,
    )?;

    // --- graph_nodes: nodes in the decision/knowledge graph ---
    // A node is identified by URI. kind = "decision" | "file" | "concept" | etc.
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS graph_nodes (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    uri         TEXT UNIQUE NOT NULL,
    kind        TEXT NOT NULL,
    title       TEXT,
    first_seen  INTEGER NOT NULL,
    last_seen   INTEGER NOT NULL,
    meta        TEXT
);
CREATE INDEX IF NOT EXISTS idx_graph_nodes_kind ON graph_nodes(kind);
"#,
    )?;

    // --- graph_edges: directed edges between nodes ---
    // rel = "caused" | "depends_on" | "derived_from" | "supersedes" | "related"
    // weight = confidence / strength (0.0 - 1.0, default 1.0)
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS graph_edges (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    from_uri    TEXT NOT NULL,
    to_uri      TEXT NOT NULL,
    rel         TEXT NOT NULL,
    weight      REAL NOT NULL DEFAULT 1.0,
    ts          INTEGER NOT NULL,
    session_id  TEXT,
    agent       TEXT,
    meta        TEXT,
    FOREIGN KEY (from_uri) REFERENCES graph_nodes(uri) ON DELETE CASCADE,
    FOREIGN KEY (to_uri) REFERENCES graph_nodes(uri) ON DELETE CASCADE,
    UNIQUE (from_uri, to_uri, rel)
);
CREATE INDEX IF NOT EXISTS idx_graph_edges_from ON graph_edges(from_uri);
CREATE INDEX IF NOT EXISTS idx_graph_edges_to ON graph_edges(to_uri);
CREATE INDEX IF NOT EXISTS idx_graph_edges_to_rel ON graph_edges(to_uri, rel);
CREATE INDEX IF NOT EXISTS idx_graph_edges_from_rel ON graph_edges(from_uri, rel);
CREATE INDEX IF NOT EXISTS idx_graph_edges_rel ON graph_edges(rel);
"#,
    )?;

    // --- token_cost: per-call token usage for cost analytics ---
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS token_cost (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    ts              INTEGER NOT NULL,
    session_id      TEXT,
    agent           TEXT NOT NULL,
    model           TEXT NOT NULL,
    input_tokens    INTEGER NOT NULL DEFAULT 0,
    output_tokens   INTEGER NOT NULL DEFAULT 0,
    cache_read      INTEGER NOT NULL DEFAULT 0,
    cache_write     INTEGER NOT NULL DEFAULT 0,
    cost_usd        REAL NOT NULL DEFAULT 0.0,
    meta            TEXT
);
CREATE INDEX IF NOT EXISTS idx_token_cost_ts ON token_cost(ts);
CREATE INDEX IF NOT EXISTS idx_token_cost_agent_ts ON token_cost(agent, ts);
CREATE INDEX IF NOT EXISTS idx_token_cost_model_ts ON token_cost(model, ts);
CREATE INDEX IF NOT EXISTS idx_token_cost_session ON token_cost(session_id);
"#,
    )?;

    // --- why_chain view: recursive CTE for decision-chain traversal ---
    // Returns (uri, kind, depth, path) for a given starting URI.
    // depth 0 = the starting node, depth 1 = direct causes, etc.
    conn.execute_batch(
        r#"
CREATE VIEW IF NOT EXISTS why_chain AS
WITH RECURSIVE chain(uri, kind, depth, path) AS (
    SELECT uri, kind, 0, uri
    FROM graph_nodes
    UNION ALL
    SELECT e.from_uri, n.kind, c.depth + 1, c.path || ' <- ' || e.from_uri
    FROM chain c
    JOIN graph_edges e ON e.to_uri = c.uri
    JOIN graph_nodes n ON n.uri = e.from_uri
    WHERE c.depth < 20 AND e.rel IN ('caused', 'derived_from', 'depends_on')
)
SELECT * FROM chain;
"#,
    )?;

    // --- graph_expand view: recursive CTE for forward graph traversal ---
    conn.execute_batch(
        r#"
CREATE VIEW IF NOT EXISTS graph_expand AS
WITH RECURSIVE expand(uri, depth, path) AS (
    SELECT uri, 0, uri FROM graph_nodes
    UNION ALL
    SELECT e.to_uri, ex.depth + 1, ex.path || ' -> ' || e.to_uri
    FROM expand ex
    JOIN graph_edges e ON e.from_uri = ex.uri
    WHERE ex.depth < 20
)
SELECT * FROM expand;
"#,
    )?;

    // --- auto-graph trigger: when a decision is inserted, create nodes + edges ---
    conn.execute_batch(
        r#"
CREATE TRIGGER IF NOT EXISTS trg_decision_to_graph
AFTER INSERT ON decisions
WHEN new.source_uri IS NOT NULL AND new.target_uri IS NOT NULL
BEGIN
    INSERT OR IGNORE INTO graph_nodes (uri, kind, title, first_seen, last_seen)
    VALUES (new.uri, 'decision', new.rationale, new.ts, new.ts);
    INSERT OR IGNORE INTO graph_nodes (uri, kind, title, first_seen, last_seen)
    VALUES (new.source_uri, 'source', NULL, new.ts, new.ts);
    INSERT OR IGNORE INTO graph_nodes (uri, kind, title, first_seen, last_seen)
    VALUES (new.target_uri, 'target', NULL, new.ts, new.ts);
    INSERT OR IGNORE INTO graph_edges (from_uri, to_uri, rel, weight, ts, session_id, agent)
    VALUES (new.source_uri, new.uri, 'caused', 1.0, new.ts, new.session_id, new.agent);
    INSERT OR IGNORE INTO graph_edges (from_uri, to_uri, rel, weight, ts, session_id, agent)
    VALUES (new.uri, new.target_uri, 'derived_from', 1.0, new.ts, new.session_id, new.agent);
    UPDATE graph_nodes SET last_seen = new.ts WHERE uri IN (new.uri, new.source_uri, new.target_uri);
END;
"#,
    )?;

    // --- FTS5 full-text index on synapse_events.content ---
    // External-content FTS5: the index references rows in synapse_events by
    // rowid, so dedup / compression / existing indexes stay untouched. Triggers
    // keep the index in sync on INSERT / UPDATE / DELETE. Query via
    // `events::search_events`.
    conn.execute_batch(
        r#"
CREATE VIRTUAL TABLE IF NOT EXISTS synapse_events_fts
USING fts5(content, content='synapse_events', content_rowid='id', tokenize='porter unicode61');

CREATE TRIGGER IF NOT EXISTS trg_events_ai_fts
AFTER INSERT ON synapse_events
WHEN new.content IS NOT NULL
BEGIN
    INSERT INTO synapse_events_fts(rowid, content) VALUES (new.id, new.content);
END;

CREATE TRIGGER IF NOT EXISTS trg_events_ad_fts
AFTER DELETE ON synapse_events
WHEN old.content IS NOT NULL
BEGIN
    INSERT INTO synapse_events_fts(synapse_events_fts, rowid, content) VALUES ('delete', old.id, old.content);
END;

CREATE TRIGGER IF NOT EXISTS trg_events_au_fts
AFTER UPDATE ON synapse_events
WHEN old.content IS NOT NULL OR new.content IS NOT NULL
BEGIN
    INSERT INTO synapse_events_fts(synapse_events_fts, rowid, content) VALUES ('delete', old.id, old.content);
    INSERT INTO synapse_events_fts(rowid, content) VALUES (new.id, new.content);
END;
"#,
    )?;

    // Record schema version in the meta table (if it exists from synapse-core,
    // we use a separate key to avoid clashing with synapse-core's schema_version).
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS meta (
    k TEXT PRIMARY KEY,
    v TEXT NOT NULL
);
INSERT OR IGNORE INTO meta(k, v) VALUES ('ultra_schema_version', '3');
"#,
    )?;

    // PRAGMA optimize (SQLite 3.43+) — records query-planner hints in
    // sqlite_stat1 so subsequent connections pick better plans. Cheap to
    // run at end of migration; no-op on versions that predate it.
    let _ = conn.pragma_update(None, "optimize", 0_i64);

    // --- bi-temporal columns (v3): valid_time + transaction_time on decisions + events ---
    migrate_bitemporal(conn)?;

    Ok(())
}

/// Check whether a column exists on a table.
fn column_exists(conn: &Connection, table: &str, col: &str) -> bool {
    let mut stmt = match conn.prepare(&format!("PRAGMA table_info({table})")) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let mut rows = match stmt.query([]) {
        Ok(r) => r,
        Err(_) => return false,
    };
    while let Some(row) = rows.next().ok().flatten() {
        let name: String = row.get(1).unwrap_or_default();
        if name == col {
            return true;
        }
    }
    false
}

/// Add `valid_time` and `transaction_time` columns to `decisions` and
/// `synapse_events`, plus a `why_at` view for bi-temporal queries. Idempotent.
pub fn migrate_bitemporal(conn: &Connection) -> UltraResult<()> {
    // decisions: valid_time (when the fact was true), transaction_time (when stored)
    if !column_exists(conn, "decisions", "valid_time") {
        conn.execute_batch("ALTER TABLE decisions ADD COLUMN valid_time INTEGER;")?;
    }
    if !column_exists(conn, "decisions", "transaction_time") {
        conn.execute_batch("ALTER TABLE decisions ADD COLUMN transaction_time INTEGER;")?;
    }
    if !column_exists(conn, "synapse_events", "valid_time") {
        conn.execute_batch("ALTER TABLE synapse_events ADD COLUMN valid_time INTEGER;")?;
    }
    if !column_exists(conn, "synapse_events", "transaction_time") {
        conn.execute_batch("ALTER TABLE synapse_events ADD COLUMN transaction_time INTEGER;")?;
    }
    // Backfill: rows without valid_time/transaction_time default to ts.
    conn.execute_batch(
        "UPDATE decisions SET valid_time = ts WHERE valid_time IS NULL;
         UPDATE decisions SET transaction_time = ts WHERE transaction_time IS NULL;
         UPDATE synapse_events SET valid_time = ts WHERE valid_time IS NULL;
         UPDATE synapse_events SET transaction_time = ts WHERE transaction_time IS NULL;",
    )?;
    // Indexes for temporal queries.
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_decisions_valid_time ON decisions(valid_time);
         CREATE INDEX IF NOT EXISTS idx_decisions_tx_time ON decisions(transaction_time);
         CREATE INDEX IF NOT EXISTS idx_events_valid_time ON synapse_events(valid_time);
         CREATE INDEX IF NOT EXISTS idx_events_tx_time ON synapse_events(transaction_time);",
    )?;
    // why_at view: bi-temporal decision chain. Filter by `at` (valid_time <= at
    // AND transaction_time <= at) to get decisions that were both true AND known
    // at the given time.
    conn.execute_batch(
        r#"
CREATE VIEW IF NOT EXISTS why_at AS
WITH RECURSIVE chain(uri, kind, depth, path, valid_time, transaction_time) AS (
    SELECT uri, kind, 0, uri, first_seen, first_seen
    FROM graph_nodes
    UNION ALL
    SELECT e.from_uri, n.kind, c.depth + 1, c.path || ' <- ' || e.from_uri,
           e.ts, e.ts
    FROM chain c
    JOIN graph_edges e ON e.to_uri = c.uri
    JOIN graph_nodes n ON n.uri = e.from_uri
    WHERE c.depth < 20 AND e.rel IN ('caused', 'derived_from', 'depends_on')
)
SELECT * FROM chain;
"#,
    )?;
    // Update schema version.
    conn.execute_batch("INSERT OR REPLACE INTO meta(k, v) VALUES ('ultra_schema_version', '3');")?;
    Ok(())
}

/// Check the current Ultra schema version. Returns 0 if not yet migrated.
pub fn schema_version(conn: &Connection) -> u32 {
    conn.query_row(
        "SELECT v FROM meta WHERE k = 'ultra_schema_version'",
        [],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|s| s.parse().ok())
    .unwrap_or(0)
}
