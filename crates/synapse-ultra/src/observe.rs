//! Observe: brain stats, replay, cost analytics.
//!
//! Pure SQL queries — no mutations. Used by the `synapse-ultra` CLI and
//! (eventually) by the Astro 7 dashboard.

use crate::UltraResult;
use rusqlite::{Connection, params};
use serde::Serialize;

/// High-level brain statistics (for `synapse-ultra inspect`).
#[derive(Debug, Clone, Serialize)]
pub struct BrainStats {
    pub docs: i64,
    pub events: i64,
    pub decisions: i64,
    pub graph_nodes: i64,
    pub graph_edges: i64,
    pub sessions: i64,
    pub token_cost_rows: i64,
    pub total_cost_usd: f64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub db_size_bytes: u64,
    pub ultra_schema_version: u32,
}

/// Compute brain stats. Batched into a single scalar subquery scan so we
/// don't pay 8 separate COUNT(*) round-trips. `docs` is optional (only
/// present when layered on a synapse-core brain.db).
pub fn brain_stats(conn: &Connection) -> UltraResult<BrainStats> {
    let docs: i64 = if conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='docs' LIMIT 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .is_ok()
    {
        conn.query_row("SELECT COUNT(*) FROM docs", [], |row| row.get(0))?
    } else {
        0
    };
    let (events, decisions, graph_nodes, graph_edges, sessions, token_cost_rows) = conn.query_row(
        r#"
            SELECT
              (SELECT COUNT(*) FROM synapse_events),
              (SELECT COUNT(*) FROM decisions),
              (SELECT COUNT(*) FROM graph_nodes),
              (SELECT COUNT(*) FROM graph_edges),
              (SELECT COUNT(*) FROM sessions),
              (SELECT COUNT(*) FROM token_cost)
            "#,
        [],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        },
    )?;
    let (total_cost_usd, total_input_tokens, total_output_tokens) = conn
        .query_row(
            "SELECT COALESCE(SUM(cost_usd), 0.0), COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0) FROM token_cost",
            [],
            |row| {
                Ok((
                    row.get::<_, f64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )?;
    let db_size_bytes = conn
        .query_row("PRAGMA page_count", [], |row| row.get::<_, i64>(0))
        .ok()
        .and_then(|pages| {
            conn.query_row("PRAGMA page_size", [], |row| row.get::<_, i64>(0))
                .ok()
                .map(|size| (pages * size) as u64)
        })
        .unwrap_or(0);
    let ultra_schema_version = crate::schema::schema_version(conn);
    Ok(BrainStats {
        docs,
        events,
        decisions,
        graph_nodes,
        graph_edges,
        sessions,
        token_cost_rows,
        total_cost_usd,
        total_input_tokens,
        total_output_tokens,
        db_size_bytes,
        ultra_schema_version,
    })
}

/// One entry in a session replay (for `synapse-ultra replay <session>`).
#[derive(Debug, Clone, Serialize)]
pub struct ReplayEntry {
    pub ts: i64,
    pub kind: String,
    pub uri: Option<String>,
    pub content: Option<String>,
}

/// Replay a session chronologically. Returns events ordered by ts ASC.
pub fn replay(conn: &Connection, session_id: &str, limit: i64) -> UltraResult<Vec<ReplayEntry>> {
    let mut stmt = conn.prepare(
        "SELECT ts, kind, uri, content
         FROM synapse_events
         WHERE session_id = ?1
         ORDER BY ts ASC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![session_id, limit], |row| {
        Ok(ReplayEntry {
            ts: row.get(0)?,
            kind: row.get(1)?,
            uri: row.get(2)?,
            content: row.get(3)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// One row in a cost report (for `synapse-ultra cost`).
#[derive(Debug, Clone, Serialize)]
pub struct CostRow {
    pub bucket: String,
    pub agent: String,
    pub model: String,
    pub calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
}

/// Query token cost aggregated by (agent, model, day). `since_ts` filters rows.
pub fn cost_by_day(conn: &Connection, since_ts: i64) -> UltraResult<Vec<CostRow>> {
    let mut stmt = conn.prepare(
        "SELECT
            date(ts, 'unixepoch') AS bucket,
            agent,
            model,
            COUNT(*) AS calls,
            SUM(input_tokens) AS input_tokens,
            SUM(output_tokens) AS output_tokens,
            SUM(cost_usd) AS cost_usd
         FROM token_cost
         WHERE ts >= ?1
         GROUP BY bucket, agent, model
         ORDER BY bucket DESC, agent, model",
    )?;
    let rows = stmt.query_map(params![since_ts], |row| {
        Ok(CostRow {
            bucket: row.get(0)?,
            agent: row.get(1)?,
            model: row.get(2)?,
            calls: row.get(3)?,
            input_tokens: row.get(4)?,
            output_tokens: row.get(5)?,
            cost_usd: row.get(6)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// (name, count) row for `top_*` aggregate queries.
pub type NameCount = (String, i64);

/// Top agents by event count (for `synapse-ultra inspect`).
pub fn top_agents(conn: &Connection, limit: i64) -> UltraResult<Vec<NameCount>> {
    let mut stmt = conn.prepare(
        "SELECT agent, COUNT(*) AS c FROM synapse_events GROUP BY agent ORDER BY c DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |row| Ok((row.get(0)?, row.get(1)?)))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Top event kinds by count.
pub fn top_kinds(conn: &Connection, limit: i64) -> UltraResult<Vec<NameCount>> {
    let mut stmt = conn.prepare(
        "SELECT kind, COUNT(*) AS c FROM synapse_events GROUP BY kind ORDER BY c DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |row| Ok((row.get(0)?, row.get(1)?)))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// A single agent's activity across all sessions within a time range.
/// Used for cross-session tracing: "what did agent X do between t1 and t2?"
#[derive(Debug, Clone, Serialize)]
pub struct AgentTraceRow {
    pub ts: i64,
    pub session_id: Option<String>,
    pub kind: String,
    pub uri: Option<String>,
    pub content_preview: Option<String>,
}

/// Trace one agent across all sessions within `[since_ts, until_ts]`.
/// Returns rows ordered by ts ASC. `content_preview` is truncated to 200 chars.
pub fn agent_trace(
    conn: &Connection,
    agent: &str,
    since_ts: i64,
    until_ts: i64,
    limit: i64,
) -> UltraResult<Vec<AgentTraceRow>> {
    let mut stmt = conn.prepare(
        "SELECT ts, session_id, kind, uri, substr(content, 1, 200)
         FROM synapse_events
         WHERE agent = ?1 AND ts >= ?2 AND ts <= ?3
         ORDER BY ts ASC
         LIMIT ?4",
    )?;
    let rows = stmt.query_map(params![agent, since_ts, until_ts, limit], |row| {
        Ok(AgentTraceRow {
            ts: row.get(0)?,
            session_id: row.get(1)?,
            kind: row.get(2)?,
            uri: row.get(3)?,
            content_preview: row.get(4)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Per-agent summary within a time range (used by `daily_summary`).
#[derive(Debug, Clone, Serialize)]
pub struct AgentSummary {
    pub agent: String,
    pub events: i64,
    pub decisions: i64,
    pub sessions: i64,
    pub cost_usd: f64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub first_ts: i64,
    pub last_ts: i64,
    pub top_kinds: Vec<(String, i64)>,
    pub top_uris: Vec<(String, i64)>,
}

/// Aggregated daily summary: what happened on a given day?
/// `day_start_ts` / `day_end_ts` define the window (usually 24h).
#[derive(Debug, Clone, Serialize)]
pub struct DailySummary {
    pub day_start_ts: i64,
    pub day_end_ts: i64,
    pub total_events: i64,
    pub total_decisions: i64,
    pub total_sessions: i64,
    pub total_cost_usd: f64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub new_graph_nodes: i64,
    pub new_graph_edges: i64,
    pub agents: Vec<AgentSummary>,
    pub top_decisions: Vec<DecisionRow>,
}

/// A decision row (for daily summary top-decisions list).
#[derive(Debug, Clone, Serialize)]
pub struct DecisionRow {
    pub id: i64,
    pub ts: i64,
    pub agent: String,
    pub uri: String,
    pub rationale: Option<String>,
    pub source_uri: Option<String>,
    pub target_uri: Option<String>,
}

/// Compute a daily summary for `[day_start_ts, day_end_ts]`.
/// Aggregates events, decisions, costs, graph growth, per-agent breakdowns.
pub fn daily_summary(
    conn: &Connection,
    day_start_ts: i64,
    day_end_ts: i64,
) -> UltraResult<DailySummary> {
    // Totals
    let (total_events, total_decisions, total_sessions, new_graph_nodes, new_graph_edges) = conn
        .query_row(
            r#"
            SELECT
              (SELECT COUNT(*) FROM synapse_events WHERE ts >= ?1 AND ts <= ?2),
              (SELECT COUNT(*) FROM decisions WHERE ts >= ?1 AND ts <= ?2),
              (SELECT COUNT(DISTINCT session_id) FROM synapse_events
               WHERE ts >= ?1 AND ts <= ?2 AND session_id IS NOT NULL),
              (SELECT COUNT(*) FROM graph_nodes WHERE first_seen >= ?1 AND first_seen <= ?2),
              (SELECT COUNT(*) FROM graph_edges WHERE ts >= ?1 AND ts <= ?2)
            "#,
            params![day_start_ts, day_end_ts],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )?;
    let (total_cost_usd, total_input_tokens, total_output_tokens) = conn.query_row(
        "SELECT COALESCE(SUM(cost_usd), 0.0),
                    COALESCE(SUM(input_tokens), 0),
                    COALESCE(SUM(output_tokens), 0)
             FROM token_cost WHERE ts >= ?1 AND ts <= ?2",
        params![day_start_ts, day_end_ts],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;

    // Per-agent breakdown
    let mut stmt = conn.prepare(
        r#"
        SELECT agent,
               COUNT(*) AS events,
               COALESCE((SELECT COUNT(*) FROM decisions d
                         WHERE d.agent = e.agent AND d.ts >= ?2 AND d.ts <= ?3), 0) AS decisions,
               COUNT(DISTINCT session_id) AS sessions,
               COALESCE((SELECT SUM(cost_usd) FROM token_cost tc
                         WHERE tc.agent = e.agent AND tc.ts >= ?2 AND tc.ts <= ?3), 0.0) AS cost_usd,
               COALESCE((SELECT SUM(input_tokens) FROM token_cost tc
                         WHERE tc.agent = e.agent AND tc.ts >= ?2 AND tc.ts <= ?3), 0) AS input_tokens,
               COALESCE((SELECT SUM(output_tokens) FROM token_cost tc
                         WHERE tc.agent = e.agent AND tc.ts >= ?2 AND tc.ts <= ?3), 0) AS output_tokens,
               MIN(ts) AS first_ts,
               MAX(ts) AS last_ts
        FROM synapse_events e
        WHERE ts >= ?2 AND ts <= ?3
        GROUP BY agent
        ORDER BY events DESC
        "#,
    )?;
    let agent_rows = stmt.query_map(params![day_start_ts, day_start_ts, day_end_ts], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, f64>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, i64>(7)?,
            row.get::<_, i64>(8)?,
        ))
    })?;
    let mut agents = Vec::new();
    for r in agent_rows {
        let (
            agent,
            events,
            decisions,
            sessions,
            cost_usd,
            input_tokens,
            output_tokens,
            first_ts,
            last_ts,
        ) = r?;
        // Top kinds for this agent
        let mut kstmt = conn.prepare(
            "SELECT kind, COUNT(*) AS c FROM synapse_events
             WHERE agent = ?1 AND ts >= ?2 AND ts <= ?3
             GROUP BY kind ORDER BY c DESC LIMIT 5",
        )?;
        let top_kinds: Vec<(String, i64)> = kstmt
            .query_map(params![agent, day_start_ts, day_end_ts], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();
        // Top URIs for this agent
        let mut ustmt = conn.prepare(
            "SELECT uri, COUNT(*) AS c FROM synapse_events
             WHERE agent = ?1 AND ts >= ?2 AND ts <= ?3 AND uri IS NOT NULL
             GROUP BY uri ORDER BY c DESC LIMIT 5",
        )?;
        let top_uris: Vec<(String, i64)> = ustmt
            .query_map(params![agent, day_start_ts, day_end_ts], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();
        agents.push(AgentSummary {
            agent,
            events,
            decisions,
            sessions,
            cost_usd,
            input_tokens,
            output_tokens,
            first_ts,
            last_ts,
            top_kinds,
            top_uris,
        });
    }

    // Top decisions (most recent first, limit 20)
    let mut dstmt = conn.prepare(
        "SELECT id, ts, agent, uri, rationale, source_uri, target_uri
         FROM decisions WHERE ts >= ?1 AND ts <= ?2
         ORDER BY ts DESC LIMIT 20",
    )?;
    let top_decisions: Vec<DecisionRow> = dstmt
        .query_map(params![day_start_ts, day_end_ts], |row| {
            Ok(DecisionRow {
                id: row.get(0)?,
                ts: row.get(1)?,
                agent: row.get(2)?,
                uri: row.get(3)?,
                rationale: row.get(4)?,
                source_uri: row.get(5)?,
                target_uri: row.get(6)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();

    Ok(DailySummary {
        day_start_ts,
        day_end_ts,
        total_events,
        total_decisions,
        total_sessions,
        total_cost_usd,
        total_input_tokens,
        total_output_tokens,
        new_graph_nodes,
        new_graph_edges,
        agents,
        top_decisions,
    })
}

/// A timeline row for a single session (chronological).
#[derive(Debug, Clone, Serialize)]
pub struct SessionTimelineRow {
    pub ts: i64,
    pub kind: String,
    pub agent: String,
    pub uri: Option<String>,
    pub content_preview: Option<String>,
    pub is_decision: bool,
}

/// Build a chronological timeline for a session: events + decisions merged,
/// ordered by ts ASC. `content_preview` truncated to 200 chars.
pub fn session_timeline(
    conn: &Connection,
    session_id: &str,
    limit: i64,
) -> UltraResult<Vec<SessionTimelineRow>> {
    // Events
    let mut estmt = conn.prepare(
        "SELECT ts, kind, agent, uri, substr(content, 1, 200)
         FROM synapse_events
         WHERE session_id = ?1
         ORDER BY ts ASC
         LIMIT ?2",
    )?;
    let mut out: Vec<SessionTimelineRow> = estmt
        .query_map(params![session_id, limit], |row| {
            Ok(SessionTimelineRow {
                ts: row.get(0)?,
                kind: row.get(1)?,
                agent: row.get(2)?,
                uri: row.get(3)?,
                content_preview: row.get(4)?,
                is_decision: false,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    // Decisions (mark is_decision = true)
    let mut dstmt = conn.prepare(
        "SELECT ts, agent, uri, substr(rationale, 1, 200)
         FROM decisions
         WHERE session_id = ?1
         ORDER BY ts ASC
         LIMIT ?2",
    )?;
    let decs: Vec<SessionTimelineRow> = dstmt
        .query_map(params![session_id, limit], |row| {
            Ok(SessionTimelineRow {
                ts: row.get(0)?,
                kind: "decision".into(),
                agent: row.get(1)?,
                uri: row.get(2)?,
                content_preview: row.get(3)?,
                is_decision: true,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    out.extend(decs);
    out.sort_by_key(|r| r.ts);
    if out.len() as i64 > limit {
        out.truncate(limit as usize);
    }
    Ok(out)
}

/// List all sessions with event counts, agent, first/last ts, cost.
/// For "what sessions exist and what did they do?"
#[derive(Debug, Clone, Serialize)]
pub struct SessionRow {
    pub session_id: String,
    pub agent: String,
    pub events: i64,
    pub decisions: i64,
    pub first_ts: i64,
    pub last_ts: i64,
    pub cost_usd: f64,
}

/// List sessions ordered by last_ts DESC. Filters by agent if provided.
pub fn list_sessions(
    conn: &Connection,
    agent_filter: Option<&str>,
    limit: i64,
) -> UltraResult<Vec<SessionRow>> {
    let sql = if agent_filter.is_some() {
        r#"
        SELECT s.session_id, s.agent,
               (SELECT COUNT(*) FROM synapse_events e WHERE e.session_id = s.session_id) AS events,
               (SELECT COUNT(*) FROM decisions d WHERE d.session_id = s.session_id) AS decisions,
               s.started_at, COALESCE(s.ended_at, s.started_at),
               COALESCE((SELECT SUM(cost_usd) FROM token_cost tc WHERE tc.session_id = s.session_id), 0.0)
        FROM sessions s
        WHERE s.agent = ?1
        ORDER BY COALESCE(s.ended_at, s.started_at) DESC
        LIMIT ?2
        "#
    } else {
        r#"
        SELECT s.session_id, s.agent,
               (SELECT COUNT(*) FROM synapse_events e WHERE e.session_id = s.session_id) AS events,
               (SELECT COUNT(*) FROM decisions d WHERE d.session_id = s.session_id) AS decisions,
               s.started_at, COALESCE(s.ended_at, s.started_at),
               COALESCE((SELECT SUM(cost_usd) FROM token_cost tc WHERE tc.session_id = s.session_id), 0.0)
        FROM sessions s
        ORDER BY COALESCE(s.ended_at, s.started_at) DESC
        LIMIT ?1
        "#
    };
    let mut stmt = conn.prepare(sql)?;
    let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<SessionRow> {
        Ok(SessionRow {
            session_id: row.get(0)?,
            agent: row.get(1)?,
            events: row.get(2)?,
            decisions: row.get(3)?,
            first_ts: row.get(4)?,
            last_ts: row.get(5)?,
            cost_usd: row.get(6)?,
        })
    };
    let mut out = Vec::new();
    if let Some(a) = agent_filter {
        let mut rows = stmt.query(params![a, limit])?;
        while let Some(r) = rows.next()? {
            out.push(map_row(r)?);
        }
    } else {
        let mut rows = stmt.query(params![limit])?;
        while let Some(r) = rows.next()? {
            out.push(map_row(r)?);
        }
    }
    Ok(out)
}
