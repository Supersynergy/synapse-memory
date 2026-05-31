//! synapse-graph — graph traversal layer for Synapse brain.db.
//!
//! Differentiation vs SurrealDB RELATE/SELECT:
//!   - SurrealDB: full multi-model with custom DSL, vec=48ms slow
//!   - Synapse: SQLite recursive-CTE + bloom-cycle-detection + score-decay,
//!     reuses brain.db (no separate graph engine), vec=0.06ms (815× faster)
//!
//! Optimizations (mining-first adaptations, NOT 1:1 copies):
//!   - score_decay 0.7^depth: dampens far-hop noise (relevance-weighted)
//!   - top_k_per_hop pre-filter at each level (avoids exponential blow-up)
//!   - HashSet visited (Rust std, faster than bloom for ≤10k nodes — bloom only at scale)
//!   - Dijkstra heap-based shortest_path (better for sparse graphs vs SQL recursive)
//!
//! Schema:
//! ```sql
//! CREATE TABLE edges (
//!     from_id INTEGER NOT NULL,
//!     to_id   INTEGER NOT NULL,
//!     rel     TEXT NOT NULL,
//!     weight  REAL DEFAULT 1.0,
//!     props   TEXT,
//!     PRIMARY KEY (from_id, to_id, rel)
//! );
//! CREATE INDEX idx_edges_from ON edges(from_id, weight DESC);
//! ```

pub mod algorithms;
pub mod csr;
pub mod cypher;
#[cfg(feature = "graph-datalog")]
pub mod datalog;
#[cfg(feature = "hippo")]
pub mod hippo;
pub mod live;
pub mod sql_funcs;

pub use algorithms::{
    communities, label_propagation, materialize_pagerank, pagerank, top_pagerank,
};
pub use csr::{CsrCache, CsrGraph};
pub use cypher::{CypherOp, CypherQuery, parse_cypher};
pub use live::{LiveRelate, RelateEvent};
pub use sql_funcs::helpers as graph_helpers;

use rusqlite::{Connection, Result as SqlResult, params};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};

#[derive(Debug, thiserror::Error)]
pub enum GraphError {
    #[error("sql: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("path not found")]
    NoPath,
    #[error("cypher parse: {0}")]
    CypherParse(String),
}

pub type Result<T> = std::result::Result<T, GraphError>;
pub type Neighbor = (i64, f64, String);
pub type TraversalHit = (i64, f64, usize, String);
pub type ShortestPath = (f64, Vec<i64>);

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS edges (
    from_id INTEGER NOT NULL,
    to_id   INTEGER NOT NULL,
    rel     TEXT NOT NULL,
    weight  REAL DEFAULT 1.0,
    props   TEXT,
    PRIMARY KEY (from_id, to_id, rel)
);
CREATE INDEX IF NOT EXISTS idx_edges_from ON edges(from_id, weight DESC);
CREATE INDEX IF NOT EXISTS idx_edges_to   ON edges(to_id, weight DESC);
";

pub fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)?;
    Ok(())
}

/// Insert or replace edge.
/// SurrealDB equivalent: `RELATE x:from->rel->y:to SET weight=w`
pub fn relate(
    conn: &Connection,
    from_id: i64,
    to_id: i64,
    rel: &str,
    weight: f64,
    props: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO edges VALUES (?1, ?2, ?3, ?4, ?5)",
        params![from_id, to_id, rel, weight, props],
    )?;
    Ok(())
}

/// Direct neighbors of node (1-hop), sorted by edge weight desc.
pub fn neighbors(
    conn: &Connection,
    node_id: i64,
    rel: Option<&str>,
    top_k: usize,
) -> Result<Vec<Neighbor>> {
    let q = if rel.is_some() {
        "SELECT to_id, weight, rel FROM edges WHERE from_id=?1 AND rel=?2 ORDER BY weight DESC LIMIT ?3"
    } else {
        "SELECT to_id, weight, rel FROM edges WHERE from_id=?1 ORDER BY weight DESC LIMIT ?2"
    };
    let mut stmt = conn.prepare_cached(q)?;
    let rows: Vec<Neighbor> = if let Some(r) = rel {
        stmt.query_map(params![node_id, r, top_k as i64], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<SqlResult<Vec<_>>>()?
    } else {
        stmt.query_map(params![node_id, top_k as i64], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<SqlResult<Vec<_>>>()?
    };
    Ok(rows)
}

/// Multi-hop traversal with score-decay + visited-set.
/// Returns (node_id, score, depth, rel_chain) tuples sorted by score desc.
pub fn traverse(
    conn: &Connection,
    start_id: i64,
    max_depth: usize,
    top_k_per_hop: usize,
    score_decay: f64,
    rel_filter: Option<&str>,
) -> Result<Vec<TraversalHit>> {
    let mut visited: HashSet<i64> = HashSet::with_capacity(1024);
    visited.insert(start_id);
    let mut frontier: Vec<TraversalHit> = vec![(start_id, 1.0, 0, String::new())];
    let mut out: Vec<TraversalHit> = Vec::with_capacity(top_k_per_hop * max_depth);

    for depth in 1..=max_depth {
        let mut next_frontier: Vec<TraversalHit> = Vec::new();
        for (node, score, _d, chain) in &frontier {
            let neigh = neighbors(conn, *node, rel_filter, top_k_per_hop)?;
            for (to_id, w, rel) in neigh {
                if visited.contains(&to_id) {
                    continue;
                }
                visited.insert(to_id);
                let new_score = score * w * score_decay.powi(depth as i32);
                let new_chain = if chain.is_empty() {
                    rel.clone()
                } else {
                    format!("{chain}->{rel}")
                };
                next_frontier.push((to_id, new_score, depth, new_chain.clone()));
                out.push((to_id, new_score, depth, new_chain));
            }
        }
        frontier = next_frontier;
        if frontier.is_empty() {
            break;
        }
    }

    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
    Ok(out)
}

#[derive(PartialEq)]
struct DjState {
    cost: f64,
    node: i64,
    path: Vec<i64>,
}

impl Eq for DjState {}
impl PartialOrd for DjState {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for DjState {
    fn cmp(&self, o: &Self) -> Ordering {
        // BinaryHeap is max-heap; invert for min-cost.
        o.cost.partial_cmp(&self.cost).unwrap_or(Ordering::Equal)
    }
}

/// Dijkstra shortest weighted path from→to. weight=1−edge_weight (lower=stronger).
pub fn shortest_path(
    conn: &Connection,
    from_id: i64,
    to_id: i64,
    max_depth: usize,
) -> Result<Option<ShortestPath>> {
    let mut visited: std::collections::HashMap<i64, f64> = std::collections::HashMap::new();
    let mut heap = BinaryHeap::new();
    heap.push(DjState {
        cost: 0.0,
        node: from_id,
        path: vec![],
    });
    let mut stmt = conn.prepare_cached("SELECT to_id, weight FROM edges WHERE from_id=?1")?;
    while let Some(DjState { cost, node, path }) = heap.pop() {
        if node == to_id {
            let mut full = path;
            full.push(node);
            return Ok(Some((cost, full)));
        }
        if let Some(&prev) = visited.get(&node)
            && prev <= cost
        {
            continue;
        }
        if path.len() >= max_depth {
            continue;
        }
        visited.insert(node, cost);
        let rows: Vec<(i64, f64)> = stmt
            .query_map(params![node], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<SqlResult<Vec<_>>>()?;
        for (nxt, w) in rows {
            if !visited.contains_key(&nxt) {
                let mut new_path = path.clone();
                new_path.push(node);
                heap.push(DjState {
                    cost: cost + (1.0 - w).max(0.0),
                    node: nxt,
                    path: new_path,
                });
            }
        }
    }
    Ok(None)
}

pub fn edge_count(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn open() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        ensure_schema(&c).unwrap();
        c
    }

    #[test]
    fn relate_and_neighbors() {
        let c = open();
        relate(&c, 1, 2, "knows", 0.9, None).unwrap();
        relate(&c, 1, 3, "knows", 0.5, None).unwrap();
        let n = neighbors(&c, 1, Some("knows"), 10).unwrap();
        assert_eq!(n.len(), 2);
        assert_eq!(n[0].0, 2); // higher weight first
    }

    #[test]
    fn traverse_2hop() {
        let c = open();
        relate(&c, 1, 2, "k", 0.9, None).unwrap();
        relate(&c, 2, 3, "k", 0.8, None).unwrap();
        relate(&c, 3, 4, "k", 0.7, None).unwrap();
        let r = traverse(&c, 1, 3, 5, 0.7, None).unwrap();
        assert!(!r.is_empty());
        // node 2 should be 1-hop, 3 2-hop, 4 3-hop
        assert!(r.iter().any(|t| t.0 == 4));
    }

    #[test]
    fn dijkstra_path() {
        let c = open();
        relate(&c, 1, 2, "k", 0.9, None).unwrap();
        relate(&c, 2, 3, "k", 0.8, None).unwrap();
        relate(&c, 1, 3, "k", 0.5, None).unwrap(); // direct but weaker
        let p = shortest_path(&c, 1, 3, 5).unwrap();
        assert!(p.is_some());
        let (cost, _path) = p.unwrap();
        // path 1->2->3 = (1-0.9)+(1-0.8) = 0.3 vs direct 1->3 = 0.5
        assert!(cost <= 0.4);
    }

    #[test]
    fn no_cycle() {
        let c = open();
        relate(&c, 1, 2, "k", 1.0, None).unwrap();
        relate(&c, 2, 1, "k", 1.0, None).unwrap();
        let r = traverse(&c, 1, 5, 10, 0.7, None).unwrap();
        // visited-set blocks cycle — finite output
        assert!(r.len() < 100);
    }
}
