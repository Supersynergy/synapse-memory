//! HippoRAG-2 / GraphRAG retrieval pattern for synapse-graph.
//!
//! Pipeline:
//!   1. `build_kg_from_docs`  — regex NER (capitalized noun-phrases) → entities +
//!      co-occurrence edges inserted into SOTA tables (entities / memories /
//!      memory_edges). Idempotent via INSERT OR IGNORE.
//!   2. `personalized_pagerank` — re-exported from synapse-core::ppr (PPR over
//!      memory_edges, seeds = query entity hits).
//!   3. `hippo_retrieve` — query → entity seeds → PPR → doc_ids.
//!   4. `rrf_hippo` — RRF merge of a vec-hybrid hit list with hippo doc scores.
//!
//! Feature-gated: compile with `--features hippo` (default off).
//!
//! Minimal NER uses a Regex for `\b([A-Z][a-z]+(?:\s+[A-Z][a-z]+)*)\b`
//! (capitalized multi-word noun phrases). Production upgrade path: swap
//! `extract_entities_regex` for an Ollama/LLM call returning JSON spans.

use rusqlite::{Connection, Result as SqlResult, params};
use std::collections::HashMap;

/// Doc identifier (maps to `docs.id`).
pub type DocId = i64;
/// Node identifier (maps to `memories.id`).
pub type NodeId = i64;
pub type RankedDoc = (DocId, f64);
pub type RankedNode = (NodeId, f64);

// ── Regex NER ──────────────────────────────────────────────────────────────

/// Extract candidate entity strings from text via capitalized-phrase regex.
/// Returns deduplicated entity surface forms.
pub fn extract_entities_regex(text: &str) -> Vec<String> {
    // Match capitalized sequences: "Albert Einstein", "General Relativity"
    let re = regex::Regex::new(r"\b([A-Z][a-z]+(?:\s+[A-Z][a-z]+)*)\b").unwrap();
    let mut seen = std::collections::HashSet::new();
    re.find_iter(text)
        .map(|m| m.as_str().to_string())
        .filter(|s| s.len() >= 3 && seen.insert(s.clone()))
        .collect()
}

// ── KG Build ───────────────────────────────────────────────────────────────

/// Build or extend the knowledge graph from `(doc_id, text)` pairs.
///
/// For each doc:
///   1. Extract entity surface forms.
///   2. INSERT OR IGNORE entity into `entities`.
///   3. INSERT OR IGNORE a `memories` row linking entity to doc.
///   4. For every pair of entities in same doc → INSERT OR IGNORE
///      co-occurrence edge (bidirectional) in `memory_edges`.
///
/// Uses FK-safe inserts: ensures `memories.doc_id` references a row in
/// `docs`. Caller is responsible that `docs` rows exist before calling.
/// Requires SOTA schema (`sota_migrate` called on conn).
pub fn build_kg_from_docs(conn: &Connection, docs: &[(DocId, &str)]) -> SqlResult<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    for (doc_id, text) in docs {
        let entities = extract_entities_regex(text);

        let mut memory_ids: Vec<i64> = Vec::with_capacity(entities.len());

        for ent in &entities {
            // Upsert entity.
            conn.execute(
                "INSERT OR IGNORE INTO entities (canonical_name, entity_type, created_ts)
                 VALUES (?1, 'NP', ?2)",
                params![ent, now],
            )?;
            let entity_id: i64 = conn.query_row(
                "SELECT id FROM entities WHERE canonical_name = ?1",
                params![ent],
                |r| r.get(0),
            )?;

            // Upsert memory linking entity ↔ doc.
            conn.execute(
                "INSERT OR IGNORE INTO memories
                 (doc_id, memory_type, entity_id, weight, confidence, created_ts, updated_ts)
                 VALUES (?1, 'fact', ?2, 1.0, 1.0, ?3, ?3)",
                params![doc_id, entity_id, now],
            )?;
            let memory_id: i64 = conn.query_row(
                "SELECT id FROM memories WHERE doc_id = ?1 AND entity_id = ?2",
                params![doc_id, entity_id],
                |r| r.get(0),
            )?;
            memory_ids.push(memory_id);
        }

        // Co-occurrence edges within same doc (weight inversely proportional to
        // # entities = stronger signal when fewer entities).
        let n = memory_ids.len();
        if n < 2 {
            continue;
        }
        let co_weight = 1.0_f64 / (n as f64).sqrt();
        for i in 0..n {
            for j in (i + 1)..n {
                let a = memory_ids[i];
                let b = memory_ids[j];
                conn.execute(
                    "INSERT OR IGNORE INTO memory_edges
                     (src_id, dst_id, edge_type, weight, created_ts)
                     VALUES (?1, ?2, 'co_occur', ?3, ?4)",
                    params![a, b, co_weight, now],
                )?;
                conn.execute(
                    "INSERT OR IGNORE INTO memory_edges
                     (src_id, dst_id, edge_type, weight, created_ts)
                     VALUES (?1, ?2, 'co_occur', ?3, ?4)",
                    params![b, a, co_weight, now],
                )?;
            }
        }
    }
    Ok(())
}

// ── PPR (re-export thin wrapper) ────────────────────────────────────────────

/// Personalized PageRank over `memory_edges`.
/// Seeds map memory_id → initial score (L1-normalized internally).
/// Returns (memory_id, score) pairs sorted desc, truncated to `limit`.
pub fn personalized_pagerank(
    conn: &Connection,
    seeds: &HashMap<NodeId, f64>,
    damping: f32,
    iters: usize,
    limit: usize,
) -> SqlResult<Vec<RankedNode>> {
    // inline implementation (avoids cross-crate dep on synapse-core::ppr)
    if seeds.is_empty() || iters == 0 {
        return Ok(Vec::new());
    }
    let alpha = damping as f64;
    let total: f64 = seeds.values().sum();
    let teleport: HashMap<i64, f64> = if total > 0.0 {
        seeds.iter().map(|(k, v)| (*k, v / total)).collect()
    } else {
        let n = seeds.len() as f64;
        seeds.keys().map(|k| (*k, 1.0 / n)).collect()
    };
    let mut r: HashMap<i64, f64> = teleport.clone();

    let mut stmt =
        conn.prepare_cached("SELECT dst_id, weight FROM memory_edges WHERE src_id = ?1 LIMIT 64")?;

    for _ in 0..iters {
        let mut next: HashMap<i64, f64> = HashMap::with_capacity(r.len() * 2);
        for (node, t) in &teleport {
            *next.entry(*node).or_insert(0.0) += alpha * t;
        }
        for (&node, &score) in r.iter() {
            if score < 1e-9 {
                continue;
            }
            let neigh: Vec<RankedNode> = stmt
                .query_map(params![node], |row| Ok((row.get(0)?, row.get(1)?)))?
                .filter_map(|r| r.ok())
                .collect();
            if neigh.is_empty() {
                for (n, t) in &teleport {
                    *next.entry(*n).or_insert(0.0) += (1.0 - alpha) * score * t;
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

    let mut out: Vec<RankedNode> = r.into_iter().collect();
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(limit);
    Ok(out)
}

// ── HippoRAG retrieve ──────────────────────────────────────────────────────

/// HippoRAG-2 retrieval:
///   1. NER on query text.
///   2. Look up matching `memories.id` rows (entity name LIKE).
///   3. Seed PPR with those memory_ids (uniform weight).
///   4. Map top PPR memory_ids → doc_ids, deduplicate, return top-`top_k`.
pub fn hippo_retrieve(
    conn: &Connection,
    query: &str,
    top_k: usize,
    damping: f32,
    iters: usize,
) -> SqlResult<Vec<RankedDoc>> {
    let query_entities = extract_entities_regex(query);

    if query_entities.is_empty() {
        return Ok(Vec::new());
    }

    // Find memory_ids for each query entity (fuzzy: LIKE '%entity%').
    let mut seeds: HashMap<NodeId, f64> = HashMap::new();
    for ent in &query_entities {
        let pattern = format!("%{ent}%");
        let rows: Vec<i64> = conn
            .prepare_cached(
                "SELECT m.id FROM memories m
                 JOIN entities e ON e.id = m.entity_id
                 WHERE e.canonical_name LIKE ?1
                 LIMIT 10",
            )?
            .query_map(params![pattern], |r| r.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        for id in rows {
            *seeds.entry(id).or_insert(0.0) += 1.0;
        }
    }

    if seeds.is_empty() {
        return Ok(Vec::new());
    }

    let ppr_results = personalized_pagerank(conn, &seeds, damping, iters, top_k * 4)?;

    // Map memory_ids → doc_ids, aggregate scores.
    let mut doc_scores: HashMap<DocId, f64> = HashMap::new();
    for (memory_id, score) in ppr_results {
        let doc_id: Option<i64> = conn
            .query_row(
                "SELECT doc_id FROM memories WHERE id = ?1",
                params![memory_id],
                |r| r.get(0),
            )
            .ok();
        if let Some(did) = doc_id {
            let entry = doc_scores.entry(did).or_insert(0.0);
            if score > *entry {
                *entry = score;
            }
        }
    }

    let mut out: Vec<RankedDoc> = doc_scores.into_iter().collect();
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(top_k);
    Ok(out)
}

// ── RRF merge ─────────────────────────────────────────────────────────────

/// RRF merge of a vec-hybrid ranked list with hippo graph scores.
///
/// `vec_hits`: (doc_id, score) from vec/hybrid search, rank-ordered.
/// `hippo_hits`: (doc_id, score) from `hippo_retrieve`, rank-ordered.
/// `alpha_graph`: weight of graph signal (0.0 = vec only, 1.0 = graph only).
/// Returns doc_ids sorted by merged score desc.
pub fn rrf_hippo(
    vec_hits: &[(DocId, f64)],
    hippo_hits: &[(DocId, f64)],
    alpha_graph: f32,
    limit: usize,
) -> Vec<RankedDoc> {
    const K: f64 = 60.0;
    let alpha = alpha_graph as f64;
    let beta = 1.0 - alpha;

    let mut scores: HashMap<DocId, f64> = HashMap::new();

    for (rank, (doc_id, _)) in vec_hits.iter().enumerate() {
        *scores.entry(*doc_id).or_insert(0.0) += beta * (1.0 / (K + rank as f64 + 1.0));
    }
    for (rank, (doc_id, _)) in hippo_hits.iter().enumerate() {
        *scores.entry(*doc_id).or_insert(0.0) += alpha * (1.0 / (K + rank as f64 + 1.0));
    }

    let mut out: Vec<RankedDoc> = scores.into_iter().collect();
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(limit);
    out
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
        // minimal schema subset needed
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS docs (
                id INTEGER PRIMARY KEY,
                uri TEXT,
                title TEXT,
                text TEXT NOT NULL,
                embedding BLOB,
                metadata TEXT,
                score REAL,
                created_ts INTEGER
            );
            CREATE TABLE IF NOT EXISTS entities (
                id INTEGER PRIMARY KEY,
                canonical_name TEXT NOT NULL UNIQUE,
                entity_type TEXT,
                alias_json TEXT,
                created_ts INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS memories (
                id INTEGER PRIMARY KEY,
                doc_id INTEGER NOT NULL,
                memory_type TEXT NOT NULL DEFAULT 'raw',
                entity_id INTEGER,
                weight REAL NOT NULL DEFAULT 1.0,
                confidence REAL NOT NULL DEFAULT 1.0,
                superseded_by INTEGER,
                project_tags TEXT,
                created_ts INTEGER NOT NULL,
                updated_ts INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS memory_edges (
                src_id INTEGER NOT NULL,
                dst_id INTEGER NOT NULL,
                edge_type TEXT NOT NULL,
                weight REAL NOT NULL DEFAULT 1.0,
                created_ts INTEGER NOT NULL,
                PRIMARY KEY (src_id, dst_id, edge_type)
            );
            CREATE INDEX IF NOT EXISTS idx_edges_dst ON memory_edges(dst_id);
            "#,
        )
        .unwrap();
        conn
    }

    fn insert_doc(conn: &Connection, id: i64, text: &str) {
        conn.execute(
            "INSERT INTO docs (id, text, created_ts) VALUES (?1, ?2, 0)",
            params![id, text],
        )
        .unwrap();
    }

    // 10 synthetic docs: physics, Einstein, Bohr, collaborators, relativity.
    fn populate(conn: &Connection) {
        let docs: &[(i64, &str)] = &[
            (
                1,
                "Albert Einstein published the theory of General Relativity in 1915.",
            ),
            (
                2,
                "Niels Bohr and Albert Einstein debated quantum mechanics at Solvay.",
            ),
            (
                3,
                "Max Planck introduced quantum theory which Einstein extended.",
            ),
            (
                4,
                "Marie Curie won the Nobel Prize in Physics and Chemistry.",
            ),
            (5, "Werner Heisenberg formulated the uncertainty principle."),
            (6, "Erwin Schrodinger developed wave mechanics equations."),
            (
                7,
                "Einstein and Schrodinger exchanged letters about wave functions.",
            ),
            (
                8,
                "Bohr and Heisenberg had the Copenhagen debate on quantum interpretation.",
            ),
            (
                9,
                "General Relativity predicts gravitational lensing near massive objects.",
            ),
            (
                10,
                "Max Planck and Einstein shared views on statistical mechanics.",
            ),
        ];
        for (id, text) in docs {
            insert_doc(conn, *id, text);
        }
        let doc_refs: Vec<(i64, &str)> = docs.iter().map(|(id, t)| (*id, *t)).collect();
        build_kg_from_docs(conn, &doc_refs).unwrap();
    }

    #[test]
    fn entities_extracted_from_text() {
        let text = "Albert Einstein collaborated with Niels Bohr on quantum theory.";
        let ents = extract_entities_regex(text);
        assert!(
            ents.iter().any(|e| e.contains("Einstein")),
            "expected Einstein: {:?}",
            ents
        );
        assert!(
            ents.iter().any(|e| e.contains("Bohr")),
            "expected Bohr: {:?}",
            ents
        );
    }

    #[test]
    fn build_kg_populates_entities_and_memories() {
        let conn = setup_db();
        insert_doc(&conn, 1, "Albert Einstein published General Relativity.");
        build_kg_from_docs(
            &conn,
            &[(1, "Albert Einstein published General Relativity.")],
        )
        .unwrap();
        let ent_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0))
            .unwrap();
        assert!(ent_count >= 1, "no entities inserted");
        let mem_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memories WHERE doc_id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(mem_count >= 1, "no memories for doc 1");
    }

    #[test]
    fn build_kg_creates_cooccurrence_edges() {
        let conn = setup_db();
        insert_doc(
            &conn,
            1,
            "Albert Einstein and Niels Bohr debated at Solvay.",
        );
        build_kg_from_docs(
            &conn,
            &[(1, "Albert Einstein and Niels Bohr debated at Solvay.")],
        )
        .unwrap();
        let edge_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_edges", [], |r| r.get(0))
            .unwrap();
        assert!(
            edge_count >= 2,
            "expected bidirectional co-occur edges, got {}",
            edge_count
        );
    }

    #[test]
    fn hippo_retrieve_einstein_collaborator() {
        let conn = setup_db();
        populate(&conn);

        let results = hippo_retrieve(&conn, "Einstein collaborator", 5, 0.5, 10).unwrap();
        assert!(!results.is_empty(), "hippo_retrieve returned nothing");
        // docs 1,2,3,7,10 mention Einstein — at least one should rank in top-5
        let top_ids: Vec<i64> = results.iter().map(|(id, _)| *id).collect();
        let einstein_docs: &[i64] = &[1, 2, 3, 7, 10];
        let recall = top_ids
            .iter()
            .filter(|id| einstein_docs.contains(id))
            .count();
        assert!(recall >= 1, "no Einstein doc in top-5: {:?}", top_ids);
    }

    #[test]
    fn ppr_seeds_only_keeps_mass() {
        let conn = setup_db();
        conn.execute_batch(
            "INSERT INTO memory_edges (src_id, dst_id, edge_type, weight, created_ts)
             VALUES (1, 2, 'test', 1.0, 0);
             INSERT INTO memory_edges (src_id, dst_id, edge_type, weight, created_ts)
             VALUES (2, 1, 'test', 1.0, 0);",
        )
        .unwrap();
        let mut seeds = HashMap::new();
        seeds.insert(1i64, 1.0f64);
        let out = personalized_pagerank(&conn, &seeds, 0.5, 10, 10).unwrap();
        let total: f64 = out.iter().map(|(_, s)| s).sum();
        assert!((total - 1.0).abs() < 1e-5, "mass not conserved: {}", total);
    }

    #[test]
    fn rrf_hippo_merge_boosts_graph_signal() {
        // vec says doc 1 is best, hippo says doc 2 is best, alpha_graph=0.9
        let vec_hits = vec![(1i64, 0.9), (2i64, 0.1)];
        let hippo_hits = vec![(2i64, 0.9), (1i64, 0.1)];
        let merged = rrf_hippo(&vec_hits, &hippo_hits, 0.9, 2);
        assert!(!merged.is_empty());
        // with alpha_graph=0.9, doc 2 should win
        assert_eq!(
            merged[0].0, 2,
            "expected doc 2 to win with high alpha_graph"
        );
    }
}
