//! Personalized PageRank over `memory_edges` — HippoRAG-2 core retrieval signal.
//!
//! Adapted from osu-nlp-group/HippoRAG (Apache-2.0). Pure Rust, zero new deps,
//! works directly on rusqlite::Connection so it composes with `Store::recall`
//! without lifting the graph into memory.
//!
//! Algorithm (per HippoRAG-2 §3.2):
//!   1. Seeds = vec/FTS top-K hits with normalized scores (sum to 1).
//!   2. r0 = teleport vector = seeds.
//!   3. Iterate: r_{t+1}[v] = alpha * teleport[v] + (1-alpha) * Σ_{u→v} r_t[u] / out_deg(u)
//!   4. Stop after `iters` (HippoRAG default = 10, alpha = 0.5).
//!   5. Return ranked memory_ids by r_T.
//!
//! Why HashMap not petgraph: edge-set is sparse (typical <50k edges per active
//! window), HashMap-of-Vec is allocator-cheap, and we avoid pulling petgraph
//! into the hot dep tree. Benchmark target: <5ms for 1k seeds, depth-bounded
//! to per-iter cap of 10k visited nodes.

#![allow(clippy::type_complexity)]

use crate::error::Result;
use rusqlite::Connection;
use std::collections::HashMap;

/// HippoRAG-2 defaults: 10 iterations, alpha 0.5, neighbor cap 64/node.
pub const DEFAULT_ITERS: usize = 10;
pub const DEFAULT_ALPHA: f64 = 0.5;
pub const DEFAULT_NEIGHBOR_CAP: usize = 64;

/// Run Personalized PageRank seeded by `seeds` over the `memory_edges` graph.
///
/// `seeds` maps memory_id -> initial score (will be L1-normalized internally).
/// Returns memory_ids sorted by PPR score desc, truncated to `limit`.
pub fn personalized_pagerank(
    conn: &Connection,
    seeds: &HashMap<i64, f64>,
    alpha: f64,
    iters: usize,
    neighbor_cap: usize,
    limit: usize,
) -> Result<Vec<(i64, f64)>> {
    if seeds.is_empty() || iters == 0 {
        return Ok(Vec::new());
    }

    // L1-normalize teleport vector.
    let total: f64 = seeds.values().sum();
    let teleport: HashMap<i64, f64> = if total > 0.0 {
        seeds.iter().map(|(k, v)| (*k, v / total)).collect()
    } else {
        let n = seeds.len() as f64;
        seeds.keys().map(|k| (*k, 1.0 / n)).collect()
    };

    let mut r: HashMap<i64, f64> = teleport.clone();

    // Per-iteration: pull outgoing edges for every active node, distribute mass.
    // We re-fetch edges each iter (SQLite cached prepare → ~µs per query).
    let mut stmt =
        conn.prepare_cached("SELECT dst_id, weight FROM memory_edges WHERE src_id = ?1 LIMIT ?2")?;

    for _ in 0..iters {
        let mut next: HashMap<i64, f64> = HashMap::with_capacity(r.len() * 2);
        // Apply teleport mass first.
        for (node, t) in &teleport {
            *next.entry(*node).or_insert(0.0) += alpha * t;
        }
        // Push (1-alpha) along outgoing edges.
        for (&node, &score) in r.iter() {
            if score < 1e-9 {
                continue;
            }
            let rows = stmt.query_map(rusqlite::params![node, neighbor_cap as i64], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
            })?;
            // Collect first to compute weight sum (degree-weighted distribution).
            let neigh: Vec<(i64, f64)> = rows.filter_map(|r| r.ok()).collect();
            if neigh.is_empty() {
                // Dangling node: redistribute to teleport (HippoRAG-2 §3.2).
                for (node, t) in &teleport {
                    *next.entry(*node).or_insert(0.0) += (1.0 - alpha) * score * t;
                }
                continue;
            }
            let wsum: f64 = neigh.iter().map(|(_, w)| w).sum::<f64>().max(1e-9);
            let push = (1.0 - alpha) * score;
            for (dst, w) in neigh {
                *next.entry(dst).or_insert(0.0) += push * (w / wsum);
            }
        }
        r = next;
    }

    let mut out: Vec<(i64, f64)> = r.into_iter().collect();
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(limit);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sota::sota_migrate;

    fn fresh() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        // FK off: tests use synthetic edges without real memory rows.
        c.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
        c.execute_batch("CREATE TABLE docs (id INTEGER PRIMARY KEY, text TEXT NOT NULL);")
            .unwrap();
        sota_migrate(&c).unwrap();
        c
    }

    fn add_edge(c: &Connection, src: i64, dst: i64, w: f64) {
        c.execute(
            "INSERT OR REPLACE INTO memory_edges (src_id, dst_id, edge_type, weight, created_ts)
             VALUES (?1, ?2, 'about', ?3, 0)",
            rusqlite::params![src, dst, w],
        )
        .unwrap();
    }

    #[test]
    fn empty_seeds_returns_empty() {
        let c = fresh();
        let out = personalized_pagerank(&c, &HashMap::new(), 0.5, 10, 64, 10).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn seed_only_no_edges_keeps_mass_on_seed() {
        let c = fresh();
        let mut seeds = HashMap::new();
        seeds.insert(1, 1.0);
        let out = personalized_pagerank(&c, &seeds, 0.5, 5, 64, 10).unwrap();
        assert_eq!(out[0].0, 1);
        assert!(out[0].1 > 0.99);
    }

    #[test]
    fn mass_flows_to_neighbor() {
        let c = fresh();
        add_edge(&c, 1, 2, 1.0);
        add_edge(&c, 2, 3, 1.0);
        let mut seeds = HashMap::new();
        seeds.insert(1, 1.0);
        let out = personalized_pagerank(&c, &seeds, 0.5, 10, 64, 10).unwrap();
        // Seed should still rank highest (alpha=0.5 teleport keeps mass on it),
        // but neighbors must receive some.
        let map: HashMap<i64, f64> = out.into_iter().collect();
        assert!(map.get(&1).copied().unwrap_or(0.0) > 0.4);
        assert!(map.get(&2).copied().unwrap_or(0.0) > 0.0);
        assert!(map.get(&3).copied().unwrap_or(0.0) > 0.0);
    }

    #[test]
    fn higher_edge_weight_routes_more_mass() {
        let c = fresh();
        add_edge(&c, 1, 2, 9.0);
        add_edge(&c, 1, 3, 1.0);
        let mut seeds = HashMap::new();
        seeds.insert(1, 1.0);
        let out = personalized_pagerank(&c, &seeds, 0.5, 10, 64, 10).unwrap();
        let map: HashMap<i64, f64> = out.into_iter().collect();
        let s2 = map.get(&2).copied().unwrap_or(0.0);
        let s3 = map.get(&3).copied().unwrap_or(0.0);
        assert!(
            s2 > s3,
            "weight=9 neighbor must outrank weight=1: s2={} s3={}",
            s2,
            s3
        );
    }

    #[test]
    fn dangling_node_redistributes_to_teleport() {
        let c = fresh();
        // node 1 → 2 (no out-edges from 2). Mass should not vanish.
        add_edge(&c, 1, 2, 1.0);
        let mut seeds = HashMap::new();
        seeds.insert(1, 1.0);
        let out = personalized_pagerank(&c, &seeds, 0.5, 10, 64, 10).unwrap();
        let total: f64 = out.iter().map(|(_, s)| s).sum();
        assert!(
            (total - 1.0).abs() < 1e-6,
            "PPR mass must sum to 1.0, got {}",
            total
        );
    }
}
