//! Graph algorithms: PageRank + Louvain community detection.
//!
//! Both run in-process against `edges` table. No external graph engine needed.

use rusqlite::{Connection, params};
use std::collections::HashMap;

use crate::Result;

type WeightedEdges = HashMap<i64, Vec<(i64, f64)>>;

pub type RankedNode = (i64, f64);
pub type Community = (i64, Vec<i64>);

/// PageRank iterative computation.
///
/// Args:
/// - `damping`: 0.85 typical (Brin-Page original)
/// - `iters`: 20 typical (converges <30 for most graphs)
/// - `tol`: optional early-stop when L1-delta < tol
///
/// Returns: HashMap<node_id, score>. Scores sum ≈ 1.0.
pub fn pagerank(
    conn: &Connection,
    damping: f64,
    iters: usize,
    tol: Option<f64>,
) -> Result<HashMap<i64, f64>> {
    // Collect all node-ids from edges (union from_id + to_id)
    let nodes: Vec<i64> = {
        let mut s = std::collections::HashSet::new();
        let mut stmt = conn.prepare("SELECT from_id FROM edges UNION SELECT to_id FROM edges")?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
        for r in rows {
            s.insert(r?);
        }
        s.into_iter().collect()
    };
    let n = nodes.len();
    if n == 0 {
        return Ok(HashMap::new());
    }
    let initial = 1.0 / n as f64;
    let mut score: HashMap<i64, f64> = nodes.iter().map(|&id| (id, initial)).collect();

    // Pre-compute outgoing edges per node + outgoing weight sum
    let mut out_edges: WeightedEdges = HashMap::new();
    let mut out_sum: HashMap<i64, f64> = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT from_id, to_id, weight FROM edges")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, f64>(2)?,
            ))
        })?;
        for r in rows {
            let (f, t, w) = r?;
            out_edges.entry(f).or_default().push((t, w));
            *out_sum.entry(f).or_insert(0.0) += w;
        }
    }
    let teleport = (1.0 - damping) / n as f64;

    for _ in 0..iters {
        let mut next: HashMap<i64, f64> = nodes.iter().map(|&id| (id, teleport)).collect();
        let mut dangling = 0.0_f64;
        for (&from, &cur) in &score {
            if let Some(es) = out_edges.get(&from) {
                let total = out_sum.get(&from).copied().unwrap_or(1.0);
                for (to, w) in es {
                    let contrib = damping * cur * (w / total);
                    *next.entry(*to).or_insert(0.0) += contrib;
                }
            } else {
                // Dangling: distribute its rank to ALL nodes
                dangling += cur;
            }
        }
        if dangling > 0.0 {
            let share = damping * dangling / n as f64;
            for v in next.values_mut() {
                *v += share;
            }
        }

        if let Some(t) = tol {
            let delta: f64 = score
                .iter()
                .map(|(k, v)| (next.get(k).copied().unwrap_or(0.0) - v).abs())
                .sum();
            score = next;
            if delta < t {
                break;
            }
        } else {
            score = next;
        }
    }
    Ok(score)
}

/// Top-N nodes by PageRank score.
pub fn top_pagerank(
    conn: &Connection,
    n: usize,
    damping: f64,
    iters: usize,
) -> Result<Vec<RankedNode>> {
    let map = pagerank(conn, damping, iters, Some(1e-6))?;
    let mut v: Vec<_> = map.into_iter().collect();
    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    v.truncate(n);
    Ok(v)
}

/// Label-Propagation community detection (faster than Louvain for online updates).
///
/// Each node initially in own community. Iteratively adopts most-frequent
/// neighbor label. Converges fast (5-10 iters typical), O(E) per iter.
///
/// Returns: HashMap<node_id, community_label>.
pub fn label_propagation(conn: &Connection, max_iters: usize) -> Result<HashMap<i64, i64>> {
    let nodes: Vec<i64> = {
        let mut s = std::collections::HashSet::new();
        let mut stmt = conn.prepare("SELECT from_id FROM edges UNION SELECT to_id FROM edges")?;
        for r in stmt.query_map([], |r| r.get::<_, i64>(0))? {
            s.insert(r?);
        }
        s.into_iter().collect()
    };
    let mut labels: HashMap<i64, i64> = nodes.iter().map(|&n| (n, n)).collect();

    // Pre-compute undirected neighbor list (both from→to and to→from edges)
    let mut nbrs: WeightedEdges = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT from_id, to_id, weight FROM edges")?;
        for r in stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, f64>(2)?,
            ))
        })? {
            let (f, t, w) = r?;
            nbrs.entry(f).or_default().push((t, w));
            nbrs.entry(t).or_default().push((f, w));
        }
    }

    for _ in 0..max_iters {
        let mut changed = false;
        for &node in &nodes {
            let Some(es) = nbrs.get(&node) else { continue };
            if es.is_empty() {
                continue;
            }
            // Vote by neighbor label, weighted by edge weight
            let mut votes: HashMap<i64, f64> = HashMap::new();
            for (n, w) in es {
                let lbl = labels[n];
                *votes.entry(lbl).or_insert(0.0) += w;
            }
            let best = votes
                .into_iter()
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            if let Some((lbl, _)) = best
                && labels[&node] != lbl
            {
                labels.insert(node, lbl);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    Ok(labels)
}

/// Group nodes by community label, return sorted by size desc.
pub fn communities(conn: &Connection, max_iters: usize) -> Result<Vec<Community>> {
    let labels = label_propagation(conn, max_iters)?;
    let mut groups: HashMap<i64, Vec<i64>> = HashMap::new();
    for (node, lbl) in labels {
        groups.entry(lbl).or_default().push(node);
    }
    let mut v: Vec<_> = groups.into_iter().collect();
    v.sort_by_key(|(_, members)| std::cmp::Reverse(members.len()));
    Ok(v)
}

/// Persist PageRank scores to brain.db for fast lookups.
/// Call after batch edge updates. Reuse table for ranking-aware queries.
pub fn materialize_pagerank(conn: &Connection, damping: f64, iters: usize) -> Result<usize> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS graph_pagerank (
            node_id INTEGER PRIMARY KEY,
            score   REAL NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_pagerank_score ON graph_pagerank(score DESC);
        DELETE FROM graph_pagerank;",
    )?;
    let scores = pagerank(conn, damping, iters, Some(1e-6))?;
    let n = scores.len();
    let mut stmt = conn.prepare("INSERT INTO graph_pagerank VALUES (?1, ?2)")?;
    for (node, score) in scores {
        stmt.execute(params![node, score])?;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup(edges: &[(i64, i64, f64)]) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::ensure_schema(&conn).unwrap();
        for &(f, t, w) in edges {
            crate::relate(&conn, f, t, "REL", w, None).unwrap();
        }
        conn
    }

    #[test]
    fn pagerank_star_graph_centre_wins() {
        // Star: 1↔2, 1↔3, 1↔4 (node 1 is hub)
        let conn = setup(&[
            (1, 2, 1.0),
            (2, 1, 1.0),
            (1, 3, 1.0),
            (3, 1, 1.0),
            (1, 4, 1.0),
            (4, 1, 1.0),
        ]);
        let top = top_pagerank(&conn, 4, 0.85, 30).unwrap();
        assert_eq!(top[0].0, 1, "hub node should be top: got {top:?}");
    }

    #[test]
    fn label_propagation_two_components() {
        // Two cliques disjoint: {1,2,3} and {4,5,6}
        let conn = setup(&[
            (1, 2, 1.0),
            (2, 1, 1.0),
            (2, 3, 1.0),
            (3, 2, 1.0),
            (1, 3, 1.0),
            (3, 1, 1.0),
            (4, 5, 1.0),
            (5, 4, 1.0),
            (5, 6, 1.0),
            (6, 5, 1.0),
            (4, 6, 1.0),
            (6, 4, 1.0),
        ]);
        let groups = communities(&conn, 20).unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].1.len(), 3);
        assert_eq!(groups[1].1.len(), 3);
    }

    #[test]
    fn materialize_pagerank_persists() {
        let conn = setup(&[(1, 2, 1.0), (2, 3, 1.0), (3, 1, 1.0)]);
        let n = materialize_pagerank(&conn, 0.85, 20).unwrap();
        assert_eq!(n, 3);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM graph_pagerank", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn empty_graph_returns_empty() {
        let conn = Connection::open_in_memory().unwrap();
        crate::ensure_schema(&conn).unwrap();
        let r = pagerank(&conn, 0.85, 10, None).unwrap();
        assert_eq!(r.len(), 0);
    }
}
