//! Graph-v2: SQLite-CTE-based graph traversal.
//!
//! Replaces the broken `synapse-graph` Datalog implementation (which falls
//! over above ~100 facts). All traversal is done via recursive SQL CTEs
//! backed by `graph_nodes` + `graph_edges`. The `why()` operator is a
//! backward chain (what caused this URI?); `graph_expand()` is a forward
//! chain (what does this URI lead to?).

use crate::UltraResult;
use rusqlite::{Connection, params};
use std::collections::HashSet;

/// A node in the graph.
#[derive(Debug, Clone)]
pub struct GraphNode {
    pub uri: String,
    pub kind: String,
    pub title: Option<String>,
    pub first_seen: i64,
    pub last_seen: i64,
}

/// An edge in the graph.
#[derive(Debug, Clone)]
pub struct GraphEdge {
    pub from_uri: String,
    pub to_uri: String,
    pub rel: String,
    pub weight: f64,
    pub ts: i64,
}

/// One step in a `why()` chain. `depth` = 0 is the starting URI.
#[derive(Debug, Clone)]
pub struct WhyStep {
    pub uri: String,
    pub kind: String,
    pub depth: i64,
    pub path: String,
}

/// Upsert a graph node. Updates `last_seen` if the node already exists.
pub fn upsert_node(
    conn: &Connection,
    uri: &str,
    kind: &str,
    title: Option<&str>,
    ts: i64,
) -> UltraResult<()> {
    conn.execute(
        "INSERT INTO graph_nodes (uri, kind, title, first_seen, last_seen)
         VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT(uri) DO UPDATE SET last_seen = ?4, title = COALESCE(?3, title)",
        params![uri, kind, title, ts],
    )?;
    Ok(())
}

/// Input for [`upsert_edge`].
#[derive(Debug, Clone)]
pub struct EdgeInput<'a> {
    pub from_uri: &'a str,
    pub to_uri: &'a str,
    pub rel: &'a str,
    pub weight: f64,
    pub ts: i64,
    pub session_id: Option<&'a str>,
    pub agent: Option<&'a str>,
}

impl<'a> EdgeInput<'a> {
    /// Minimal constructor — `weight` defaults to 1.0, session/agent to `None`.
    pub fn new(from_uri: &'a str, to_uri: &'a str, rel: &'a str, ts: i64) -> Self {
        Self {
            from_uri,
            to_uri,
            rel,
            weight: 1.0,
            ts,
            session_id: None,
            agent: None,
        }
    }
}

/// Insert or update an edge. On conflict (same from/to/rel), update weight + ts.
pub fn upsert_edge(conn: &Connection, e: &EdgeInput<'_>) -> UltraResult<()> {
    conn.execute(
        "INSERT INTO graph_edges (from_uri, to_uri, rel, weight, ts, session_id, agent)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(from_uri, to_uri, rel) DO UPDATE SET weight = ?4, ts = ?5",
        params![
            e.from_uri,
            e.to_uri,
            e.rel,
            e.weight,
            e.ts,
            e.session_id,
            e.agent
        ],
    )?;
    Ok(())
}

/// `why(uri, max_depth)` — backward chain: what caused this URI?
///
/// Returns steps ordered by depth ascending (0 = the starting node, 1 = direct
/// causes, etc.). Traversal is done in Rust with a `HashSet` visited-set —
/// O(depth) cycle checks, not O(depth²) like the recursive-CTE path-string
/// approach. SQL is only used for edge lookups (indexed by `idx_graph_edges_to_rel`).
pub fn why(conn: &Connection, uri: &str, max_depth: i64) -> UltraResult<Vec<WhyStep>> {
    if max_depth <= 0 {
        return Ok(Vec::new());
    }
    // Anchor: get the starting node.
    let anchor: Option<(String, String)> = conn
        .query_row(
            "SELECT uri, kind FROM graph_nodes WHERE uri = ?1",
            params![uri],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();
    let (start_uri, start_kind) = match anchor {
        Some(v) => v,
        None => return Ok(Vec::new()),
    };

    let mut visited: HashSet<String> = HashSet::new();
    visited.insert(start_uri.clone());
    let anchor_step = WhyStep {
        uri: start_uri.clone(),
        kind: start_kind.clone(),
        depth: 0,
        path: start_uri.clone(),
    };
    let mut out = Vec::new();
    out.push(anchor_step.clone());

    // BFS frontier of WhyStep values.
    let mut frontier: Vec<WhyStep> = vec![anchor_step];

    let edge_sql = r#"
        SELECT e.from_uri, n.kind
        FROM graph_edges e
        JOIN graph_nodes n ON n.uri = e.from_uri
        WHERE e.to_uri = ?1 AND e.rel IN ('caused', 'derived_from', 'depends_on')
        ORDER BY e.from_uri ASC
    "#;

    while !frontier.is_empty() {
        let mut next: Vec<WhyStep> = Vec::new();
        for step in &frontier {
            if step.depth >= max_depth - 1 {
                continue;
            }
            // prepare_cached: rusqlite reuses the prepared statement across
            // iterations — avoids re-parsing edge_sql per BFS step.
            let mut stmt = conn.prepare_cached(edge_sql)?;
            let rows = stmt.query_map(params![step.uri], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for r in rows {
                let (n_uri, n_kind) = r?;
                if visited.insert(n_uri.clone()) {
                    let new_path = format!("{} <- {n_uri}", step.path);
                    let new_step = WhyStep {
                        uri: n_uri,
                        kind: n_kind,
                        depth: step.depth + 1,
                        path: new_path,
                    };
                    out.push(new_step.clone());
                    next.push(new_step);
                }
            }
        }
        frontier = next;
    }

    out.sort_by(|a, b| a.depth.cmp(&b.depth).then_with(|| a.uri.cmp(&b.uri)));
    Ok(out)
}

/// Bi-temporal `why`: like `why()` but only follows edges whose `ts` is <= `at`.
/// Returns the decision chain as it was known at the given timestamp.
///
/// Requires schema v3 (bi-temporal columns). On older schemas falls back to
/// `why()` (returns all edges regardless of time).
pub fn why_at(conn: &Connection, uri: &str, max_depth: i64, at: i64) -> UltraResult<Vec<WhyStep>> {
    if max_depth <= 0 {
        return Ok(Vec::new());
    }

    let anchor: Option<(String, String)> = conn
        .query_row(
            "SELECT uri, kind FROM graph_nodes WHERE uri = ?1",
            params![uri],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();
    let (start_uri, start_kind) = match anchor {
        Some(v) => v,
        None => return Ok(Vec::new()),
    };

    let mut visited: HashSet<String> = HashSet::new();
    visited.insert(start_uri.clone());
    let anchor_step = WhyStep {
        uri: start_uri.clone(),
        kind: start_kind.clone(),
        depth: 0,
        path: start_uri.clone(),
    };
    let mut out = Vec::new();
    out.push(anchor_step.clone());
    let mut frontier: Vec<WhyStep> = vec![anchor_step];

    let edge_sql = r#"
        SELECT e.from_uri, n.kind
        FROM graph_edges e
        JOIN graph_nodes n ON n.uri = e.from_uri
        WHERE e.to_uri = ?1
          AND e.rel IN ('caused', 'derived_from', 'depends_on')
          AND e.ts <= ?2
        ORDER BY e.from_uri ASC
    "#;

    while !frontier.is_empty() {
        let mut next: Vec<WhyStep> = Vec::new();
        for step in &frontier {
            if step.depth >= max_depth - 1 {
                continue;
            }
            let mut stmt = conn.prepare_cached(edge_sql)?;
            let rows = stmt.query_map(params![step.uri, at], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for r in rows {
                let (n_uri, n_kind) = r?;
                if visited.insert(n_uri.clone()) {
                    let new_path = format!("{} <- {n_uri}", step.path);
                    let new_step = WhyStep {
                        uri: n_uri,
                        kind: n_kind,
                        depth: step.depth + 1,
                        path: new_path,
                    };
                    out.push(new_step.clone());
                    next.push(new_step);
                }
            }
        }
        frontier = next;
    }

    out.sort_by(|a, b| a.depth.cmp(&b.depth).then_with(|| a.uri.cmp(&b.uri)));
    Ok(out)
}

/// `graph_expand(uri, max_depth)` — forward chain: what does this URI lead to?
/// Rust-side BFS with `HashSet` visited-set (O(depth) cycle check).
pub fn graph_expand(conn: &Connection, uri: &str, max_depth: i64) -> UltraResult<Vec<WhyStep>> {
    if max_depth <= 0 {
        return Ok(Vec::new());
    }
    let anchor: Option<(String, String)> = conn
        .query_row(
            "SELECT uri, kind FROM graph_nodes WHERE uri = ?1",
            params![uri],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();
    let (start_uri, start_kind) = match anchor {
        Some(v) => v,
        None => return Ok(Vec::new()),
    };

    let mut visited: HashSet<String> = HashSet::new();
    visited.insert(start_uri.clone());
    let anchor_step = WhyStep {
        uri: start_uri.clone(),
        kind: start_kind.clone(),
        depth: 0,
        path: start_uri.clone(),
    };
    let mut out = Vec::new();
    out.push(anchor_step.clone());
    let mut frontier: Vec<WhyStep> = vec![anchor_step];

    let edge_sql = r#"
        SELECT e.to_uri, n.kind
        FROM graph_edges e
        JOIN graph_nodes n ON n.uri = e.to_uri
        WHERE e.from_uri = ?1
        ORDER BY e.to_uri ASC
    "#;

    while !frontier.is_empty() {
        let mut next: Vec<WhyStep> = Vec::new();
        for step in &frontier {
            if step.depth >= max_depth - 1 {
                continue;
            }
            let mut stmt = conn.prepare_cached(edge_sql)?;
            let rows = stmt.query_map(params![step.uri], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for r in rows {
                let (n_uri, n_kind) = r?;
                if visited.insert(n_uri.clone()) {
                    let new_path = format!("{} -> {n_uri}", step.path);
                    let new_step = WhyStep {
                        uri: n_uri,
                        kind: n_kind,
                        depth: step.depth + 1,
                        path: new_path,
                    };
                    out.push(new_step.clone());
                    next.push(new_step);
                }
            }
        }
        frontier = next;
    }

    out.sort_by(|a, b| a.depth.cmp(&b.depth).then_with(|| a.uri.cmp(&b.uri)));
    Ok(out)
}

/// Get a node by URI.
pub fn get_node(conn: &Connection, uri: &str) -> UltraResult<Option<GraphNode>> {
    let mut stmt = conn.prepare(
        "SELECT uri, kind, title, first_seen, last_seen FROM graph_nodes WHERE uri = ?1",
    )?;
    let mut rows = stmt.query(params![uri])?;
    if let Some(row) = rows.next()? {
        return Ok(Some(GraphNode {
            uri: row.get(0)?,
            kind: row.get(1)?,
            title: row.get(2)?,
            first_seen: row.get(3)?,
            last_seen: row.get(4)?,
        }));
    }
    Ok(None)
}

/// Get all edges from a URI.
pub fn edges_from(conn: &Connection, uri: &str) -> UltraResult<Vec<GraphEdge>> {
    let mut stmt = conn.prepare(
        "SELECT from_uri, to_uri, rel, weight, ts FROM graph_edges WHERE from_uri = ?1 ORDER BY ts",
    )?;
    let rows = stmt.query_map(params![uri], |row| {
        Ok(GraphEdge {
            from_uri: row.get(0)?,
            to_uri: row.get(1)?,
            rel: row.get(2)?,
            weight: row.get(3)?,
            ts: row.get(4)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Get all edges to a URI.
pub fn edges_to(conn: &Connection, uri: &str) -> UltraResult<Vec<GraphEdge>> {
    let mut stmt = conn.prepare(
        "SELECT from_uri, to_uri, rel, weight, ts FROM graph_edges WHERE to_uri = ?1 ORDER BY ts",
    )?;
    let rows = stmt.query_map(params![uri], |row| {
        Ok(GraphEdge {
            from_uri: row.get(0)?,
            to_uri: row.get(1)?,
            rel: row.get(2)?,
            weight: row.get(3)?,
            ts: row.get(4)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Count nodes and edges.
pub fn counts(conn: &Connection) -> UltraResult<(i64, i64)> {
    let nodes: i64 = conn.query_row("SELECT COUNT(*) FROM graph_nodes", [], |row| row.get(0))?;
    let edges: i64 = conn.query_row("SELECT COUNT(*) FROM graph_edges", [], |row| row.get(0))?;
    Ok((nodes, edges))
}

/// Export the graph around a URI as Graphviz DOT format (for `synapse graph --dot`).
/// O(n) in path count — edges deduped via a HashSet.
pub fn to_dot(conn: &Connection, uri: &str, max_depth: i64) -> UltraResult<String> {
    let steps = graph_expand(conn, uri, max_depth)?;
    let mut dot = String::from("digraph synapse {\n  rankdir=LR;\n  node [shape=box];\n");
    let mut seen_nodes: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen_edges: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for s in &steps {
        if seen_nodes.insert(s.uri.clone()) {
            dot.push_str(&format!("  \"{}\" [label=\"{}\"];\n", s.uri, s.uri));
        }
        let parts: Vec<&str> = s.path.split(" -> ").collect();
        for w in parts.windows(2) {
            if seen_edges.insert((w[0].to_string(), w[1].to_string())) {
                dot.push_str(&format!("  \"{}\" -> \"{}\";\n", w[0], w[1]));
            }
        }
    }
    dot.push_str("}\n");
    Ok(dot)
}
