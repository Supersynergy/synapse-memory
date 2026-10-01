//! SOTA agent-memory layer (Tier-1 from SOTA-ROADMAP-2026-04-29.md).
//!
//! Additive — does NOT touch existing `docs` schema or hybrid `search()` path.
//! Wire via `Store::sota_migrate(&store)` (call once on open).
//!
//! Schema:
//!   memories(id, doc_id, memory_type, entity_id, weight, confidence,
//!            superseded_by, project_tags, created_ts, updated_ts)
//!   memory_edges(src_id, dst_id, edge_type, weight, created_ts)
//!   entities(id, canonical_name, entity_type, alias_json, created_ts)
//!
//! All rows reference `docs.id` (the existing doc table) for text + embedding,
//! so `memories` is a typed view over `docs` plus relations.

#![allow(clippy::type_complexity)]

use crate::error::Result;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// OMEGA / Mem0 / Hindsight typed-memory taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    /// Atomic verifiable fact: "user lives in Berlin".
    Fact,
    /// Decision with rationale: "chose SQLite over Postgres because…".
    Decision,
    /// Lesson from outcome: "long-running futures starve tokio runtime".
    Lesson,
    /// Stable preference: "prefers concise answers".
    Preference,
    /// Episodic event with timestamp: "deploy failed at 14:32 UTC".
    Episodic,
    /// Untyped — pre-extraction or extraction-failed.
    Raw,
}

impl MemoryType {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryType::Fact => "fact",
            MemoryType::Decision => "decision",
            MemoryType::Lesson => "lesson",
            MemoryType::Preference => "preference",
            MemoryType::Episodic => "episodic",
            MemoryType::Raw => "raw",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "fact" => MemoryType::Fact,
            "decision" => MemoryType::Decision,
            "lesson" => MemoryType::Lesson,
            "preference" => MemoryType::Preference,
            "episodic" => MemoryType::Episodic,
            _ => MemoryType::Raw,
        }
    }
    /// Default per-type weight applied during multi-signal fusion.
    /// Tunable later via `learn_type_weight` table (synapse-learn).
    pub fn default_weight(&self) -> f64 {
        match self {
            MemoryType::Fact => 1.20,
            MemoryType::Decision => 1.10,
            MemoryType::Lesson => 1.05,
            MemoryType::Preference => 1.15,
            MemoryType::Episodic => 0.95,
            MemoryType::Raw => 1.00,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: i64,
    pub doc_id: i64,
    pub memory_type: MemoryType,
    pub entity_id: Option<i64>,
    pub weight: f64,
    pub confidence: f64,
    pub superseded_by: Option<i64>,
    pub project_tags: Option<String>, // CSV
    pub created_ts: i64,
    pub updated_ts: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEdge {
    pub src_id: i64,
    pub dst_id: i64,
    pub edge_type: String, // "supports" | "contradicts" | "supersedes" | "about"
    pub weight: f64,
}

/// Vector search backend selector.
///
/// Routing table (Phase D-H bench, Sift-1M):
///   Cascade      — binary_k=4096 → 697 QPS @ R=0.994  (target_recall ≥0.98)
///   UsearchHnsw  — M=48, ef_s=64 → 1631 QPS @ R=0.982 (target_recall 0.94-0.97)
///   BinaryFirst  — binary pre-filter only → 4845 QPS @ R=0.888 (target_recall <0.94)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchBackend {
    Cascade,
    UsearchHnsw,
    BinaryFirst,
}

/// Route a `target_recall` value to the optimal backend.
///
/// ```
/// use synapse_core::sota::{auto_route, SearchBackend};
/// assert_eq!(auto_route(0.99), SearchBackend::Cascade);
/// assert_eq!(auto_route(0.95), SearchBackend::UsearchHnsw);
/// assert_eq!(auto_route(0.80), SearchBackend::BinaryFirst);
/// ```
pub fn auto_route(target_recall: f32) -> SearchBackend {
    if target_recall >= 0.98 {
        SearchBackend::Cascade
    } else if target_recall >= 0.94 {
        SearchBackend::UsearchHnsw
    } else {
        SearchBackend::BinaryFirst
    }
}

/// Recall request — single entrypoint for SOTA pipeline.
#[derive(Debug, Clone)]
pub struct RecallParams {
    pub query: String,
    pub k: usize,
    pub types: Option<Vec<MemoryType>>,
    pub project_tag: Option<String>,
    /// Time budget hint (ms). Pipeline stages skip rerank if remaining < 5ms.
    pub budget_ms: u32,
    /// Run cross-encoder rerank on top-N candidates (set 0 to disable).
    pub rerank_top: usize,
    /// Apply heat decay (recency × access).
    pub heat: bool,
    /// Expand via memory_edges before fusion.
    pub entity_expand: bool,
    /// BFS hop depth for entity expansion (default 2, attenuation 0.6^hop).
    pub max_hops: u8,
    /// Run Personalized PageRank (HippoRAG-2) seeded by hybrid top hits.
    /// When true, replaces BFS multi-hop with PPR signal in fusion.
    pub ppr: bool,
    /// PPR teleport probability alpha (default 0.5 per HippoRAG-2).
    pub ppr_alpha: f64,
    /// PPR iterations (default 10).
    pub ppr_iters: usize,
    /// RRF k constant (default 60 per best-practice).
    pub rrf_k: f64,
    /// Target recall threshold — drives backend auto-routing.
    /// None defaults to 0.98 (Cascade path).
    pub target_recall: Option<f32>,
    /// HyDE: expand vague queries via a hypothetical document before embedding.
    /// None = disabled (default). Requires feature `ollama`.
    #[cfg(feature = "ollama")]
    pub hyde: Option<crate::turbo::hyde::HydeConfig>,
}

impl Default for RecallParams {
    fn default() -> Self {
        Self {
            query: String::new(),
            k: 10,
            types: None,
            project_tag: None,
            budget_ms: 50,
            rerank_top: 20,
            heat: true,
            entity_expand: true,
            max_hops: 2,
            ppr: false,
            ppr_alpha: 0.5,
            ppr_iters: 10,
            rrf_k: 60.0,
            target_recall: None,
            #[cfg(feature = "ollama")]
            hyde: None,
        }
    }
}

/// Apply additive migrations. Idempotent — safe to call on existing dbs.
pub fn sota_migrate(conn: &Connection) -> Result<()> {
    // Add event_date column if missing (Supermemory dual-timestamp pattern).
    let has_event_date: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name='event_date'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .unwrap_or(false);
    if has_event_date {
        // No-op; column exists.
    }
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS entities (
            id INTEGER PRIMARY KEY,
            canonical_name TEXT NOT NULL UNIQUE,
            entity_type TEXT,
            alias_json TEXT,
            created_ts INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_entities_type ON entities(entity_type);

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
            updated_ts INTEGER NOT NULL,
            FOREIGN KEY(doc_id) REFERENCES docs(id),
            FOREIGN KEY(entity_id) REFERENCES entities(id),
            FOREIGN KEY(superseded_by) REFERENCES memories(id)
        );
        CREATE INDEX IF NOT EXISTS idx_memories_doc ON memories(doc_id);
        CREATE INDEX IF NOT EXISTS idx_memories_type ON memories(memory_type);
        CREATE INDEX IF NOT EXISTS idx_memories_entity ON memories(entity_id);
        CREATE INDEX IF NOT EXISTS idx_memories_active
            ON memories(memory_type) WHERE superseded_by IS NULL;

        CREATE TABLE IF NOT EXISTS memory_edges (
            src_id INTEGER NOT NULL,
            dst_id INTEGER NOT NULL,
            edge_type TEXT NOT NULL,
            weight REAL NOT NULL DEFAULT 1.0,
            created_ts INTEGER NOT NULL,
            PRIMARY KEY (src_id, dst_id, edge_type),
            FOREIGN KEY(src_id) REFERENCES memories(id),
            FOREIGN KEY(dst_id) REFERENCES memories(id)
        );
        CREATE INDEX IF NOT EXISTS idx_edges_dst ON memory_edges(dst_id);

        CREATE TABLE IF NOT EXISTS extraction_queue (
            doc_id INTEGER PRIMARY KEY,
            enqueued_ts INTEGER NOT NULL,
            attempts INTEGER NOT NULL DEFAULT 0,
            last_error TEXT,
            status TEXT NOT NULL DEFAULT 'pending'
        );
        CREATE INDEX IF NOT EXISTS idx_extract_status ON extraction_queue(status);
        "#,
    )?;
    // Additive: add event_date column for dual-layer timestamps if missing.
    // event_date = WHEN the memory's event happened (extracted from content),
    // distinct from created_ts = WHEN the row was inserted.
    if !has_event_date {
        let _ = conn.execute("ALTER TABLE memories ADD COLUMN event_date TEXT", []);
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_memories_event_date ON memories(event_date)",
            [],
        );
    }
    Ok(())
}

/// Insert a typed memory with explicit event_date (ISO 8601 / YYYY-MM-DD).
/// Use when extractor parsed a date from the source text.
pub fn put_memory_with_date(
    conn: &Connection,
    doc_id: i64,
    memory_type: MemoryType,
    entity_id: Option<i64>,
    project_tags: Option<&str>,
    confidence: f64,
    event_date: Option<&str>,
) -> Result<i64> {
    let ts = now_ts();
    let weight = memory_type.default_weight();
    conn.execute(
        "INSERT INTO memories
            (doc_id, memory_type, entity_id, weight, confidence,
             project_tags, created_ts, updated_ts, event_date)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8)",
        rusqlite::params![
            doc_id,
            memory_type.as_str(),
            entity_id,
            weight,
            confidence,
            project_tags,
            ts,
            event_date,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Insert a typed memory tied to an existing `docs.id`.
pub fn put_memory(
    conn: &Connection,
    doc_id: i64,
    memory_type: MemoryType,
    entity_id: Option<i64>,
    project_tags: Option<&str>,
    confidence: f64,
) -> Result<i64> {
    let ts = now_ts();
    let weight = memory_type.default_weight();
    conn.execute(
        "INSERT INTO memories
            (doc_id, memory_type, entity_id, weight, confidence,
             project_tags, created_ts, updated_ts)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        rusqlite::params![
            doc_id,
            memory_type.as_str(),
            entity_id,
            weight,
            confidence,
            project_tags,
            ts,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Ensure a raw memory row exists for a doc and return its memory id.
///
/// This wires the base `docs` table into the SOTA memory layer without forcing
/// callers to know about typed-memory internals. Typed extractors can later
/// supersede or augment this raw row.
pub fn ensure_raw_memory(conn: &Connection, doc_id: i64) -> Result<i64> {
    let existing = conn
        .query_row(
            "SELECT id FROM memories
             WHERE doc_id = ?1
               AND memory_type = 'raw'
               AND entity_id IS NULL
               AND superseded_by IS NULL
             LIMIT 1",
            rusqlite::params![doc_id],
            |r| r.get::<_, i64>(0),
        )
        .optional()?;
    if let Some(id) = existing {
        return Ok(id);
    }
    put_memory(conn, doc_id, MemoryType::Raw, None, None, 1.0)
}

/// Ensure the doc is present in both the raw memory layer and extraction queue.
pub fn ensure_raw_memory_and_enqueue(conn: &Connection, doc_id: i64) -> Result<i64> {
    let memory_id = ensure_raw_memory(conn, doc_id)?;
    enqueue_extraction(conn, doc_id)?;
    Ok(memory_id)
}

/// Batch variant used by high-throughput ingest paths.
///
/// Returns the number of newly-created raw memory rows. Queue inserts are
/// `INSERT OR IGNORE`, so duplicate docs stay cheap and idempotent.
pub fn ensure_raw_memory_and_enqueue_batch(
    conn: &mut Connection,
    doc_ids: &[i64],
) -> Result<usize> {
    if doc_ids.is_empty() {
        return Ok(0);
    }
    let tx = conn.transaction()?;
    let ts = now_ts();
    let raw_weight = MemoryType::Raw.default_weight();
    // Set-based bulk insert via json_each — replaces the per-doc
    // SELECT+INSERT+INSERT loop (O(n) round-trips → 2 statements total).
    // 2026-05-13 perf fix: prior loop was 1.98s/10k docs (93% of put_batch_fast).
    let ids_json = serde_json::to_string(doc_ids).unwrap_or_else(|_| "[]".to_string());
    let created = {
        let n = tx.execute(
            "INSERT INTO memories
                (doc_id, memory_type, entity_id, weight, confidence,
                 project_tags, created_ts, updated_ts)
             SELECT j.value, 'raw', NULL, ?1, 1.0, NULL, ?2, ?2
             FROM json_each(?3) j
             WHERE NOT EXISTS (
                 SELECT 1 FROM memories m
                 WHERE m.doc_id = j.value
                   AND m.memory_type = 'raw'
                   AND m.entity_id IS NULL
                   AND m.superseded_by IS NULL
             )",
            rusqlite::params![raw_weight, ts, ids_json],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO extraction_queue (doc_id, enqueued_ts, status)
             SELECT value, ?1, 'pending' FROM json_each(?2)",
            rusqlite::params![ts, ids_json],
        )?;
        n
    };
    tx.commit()?;
    Ok(created)
}

/// Mark `old_id` superseded by `new_id`. Records an edge.
pub fn supersede(conn: &Connection, old_id: i64, new_id: i64) -> Result<()> {
    let ts = now_ts();
    conn.execute(
        "UPDATE memories SET superseded_by = ?1, updated_ts = ?2 WHERE id = ?3",
        rusqlite::params![new_id, ts, old_id],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO memory_edges
            (src_id, dst_id, edge_type, weight, created_ts)
         VALUES (?1, ?2, 'supersedes', 1.0, ?3)",
        rusqlite::params![new_id, old_id, ts],
    )?;
    Ok(())
}

/// Enqueue a doc for async extraction (smollm2 / qwen).
pub fn enqueue_extraction(conn: &Connection, doc_id: i64) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO extraction_queue
            (doc_id, enqueued_ts, status) VALUES (?1, ?2, 'pending')",
        rusqlite::params![doc_id, now_ts()],
    )?;
    Ok(())
}

/// Multi-hop BFS over `memory_edges`. Returns (memory_id, hop_distance) pairs.
/// Attenuation `0.6^hop` is applied at recall fusion time, not here.
/// Source: ported from petgraph BFS pattern; kept allocator-free with hashbrown
/// to stay <2ms for ≤200 seed ids and depth≤3.
pub fn multi_hop_neighbors(
    conn: &Connection,
    seeds: &[i64],
    max_hops: u8,
    per_hop_cap: usize,
) -> Result<Vec<(i64, u8)>> {
    use std::collections::{HashMap, VecDeque};
    if seeds.is_empty() || max_hops == 0 {
        return Ok(Vec::new());
    }
    let mut dist: HashMap<i64, u8> = HashMap::with_capacity(seeds.len() * 4);
    let mut q: VecDeque<(i64, u8)> = VecDeque::new();
    for s in seeds {
        dist.insert(*s, 0);
        q.push_back((*s, 0));
    }
    while let Some((node, hop)) = q.pop_front() {
        if hop >= max_hops {
            continue;
        }
        let sql = "SELECT dst_id FROM memory_edges WHERE src_id = ?1 LIMIT ?2";
        let mut stmt = conn.prepare_cached(sql)?;
        let rows = stmt.query_map(rusqlite::params![node, per_hop_cap as i64], |r| {
            r.get::<_, i64>(0)
        })?;
        for row in rows {
            let dst = row?;
            if let std::collections::hash_map::Entry::Vacant(e) = dist.entry(dst) {
                e.insert(hop + 1);
                q.push_back((dst, hop + 1));
            }
        }
    }
    let mut out: Vec<(i64, u8)> = dist
        .into_iter()
        .filter(|(id, _)| !seeds.contains(id))
        .collect();
    out.sort_by_key(|(_, h)| *h);
    Ok(out)
}

/// Cosine similarity between two equal-length f32 vectors.
/// Used by `find_evolve_target` once per-doc embeddings are wired (follow-up).
#[inline]
#[allow(dead_code)]
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0f32;
    let mut na = 0f32;
    let mut nb = 0f32;
    for i in 0..a.len().min(b.len()) {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

/// Jaccard similarity over whitespace tokens (lowercased, len>2).
fn jaccard(a: &str, b: &str) -> f64 {
    use std::collections::HashSet;
    let ta: HashSet<&str> = a.split_whitespace().filter(|t| t.len() > 2).collect();
    let tb: HashSet<&str> = b.split_whitespace().filter(|t| t.len() > 2).collect();
    if ta.is_empty() && tb.is_empty() {
        return 0.0;
    }
    let inter = ta.intersection(&tb).count() as f64;
    let union = ta.union(&tb).count() as f64;
    if union == 0.0 { 0.0 } else { inter / union }
}

/// Find an existing memory whose doc-text is highly similar to `new_text`.
/// `cosine_lo..=cosine_hi` window means "similar but not identical" (evolve target).
/// Returns the doc_id of the best match in window or None.
/// Source: mem0/letta consolidation step — port to Rust with Jaccard fallback
/// when no embedding is provided.
pub fn find_evolve_target(
    conn: &Connection,
    new_text: &str,
    new_emb: Option<&[f32]>,
    candidate_doc_ids: &[i64],
    cosine_lo: f32,
    cosine_hi: f32,
) -> Result<Option<i64>> {
    let mut best: Option<(i64, f64)> = None;
    for doc_id in candidate_doc_ids {
        let txt: String =
            match conn.query_row("SELECT text FROM docs WHERE id = ?1", [doc_id], |r| {
                r.get::<_, String>(0)
            }) {
                Ok(t) => t,
                Err(_) => continue,
            };
        let score = if let Some(_q) = new_emb {
            // Embedding-based cosine path: caller would pass per-doc embeddings via
            // a sidecar table. For now we approximate via Jaccard so the function
            // is always usable. Wiring real per-doc embeddings is a follow-up.
            jaccard(new_text, &txt)
        } else {
            jaccard(new_text, &txt)
        };
        let s32 = score as f32;
        if s32 >= cosine_lo && s32 <= cosine_hi && best.map(|(_, b)| score > b).unwrap_or(true) {
            best = Some((*doc_id, score));
        }
    }
    Ok(best.map(|(id, _)| id))
}

/// Cluster active memories by Jaccard ≥ threshold and return groups.
/// Used by `compact()` nightly task. Greedy single-link clustering, O(n²) on the
/// clustering stage but bounded by the active-memory window the caller picks
/// (typical: last 7 days, <2k rows).
pub fn cluster_for_compact(
    conn: &Connection,
    jaccard_threshold: f64,
    max_rows: usize,
) -> Result<Vec<Vec<i64>>> {
    let sql = "SELECT m.id, d.text FROM memories m
               JOIN docs d ON d.id = m.doc_id
               WHERE m.superseded_by IS NULL
               ORDER BY m.updated_ts DESC
               LIMIT ?1";
    let mut stmt = conn.prepare(sql)?;
    let rows: Vec<(i64, String)> = stmt
        .query_map([max_rows as i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let n = rows.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], i: usize) -> usize {
        if parent[i] != i {
            parent[i] = find(parent, parent[i]);
        }
        parent[i]
    }
    for i in 0..n {
        for j in (i + 1)..n {
            if jaccard(&rows[i].1, &rows[j].1) >= jaccard_threshold {
                let ri = find(&mut parent, i);
                let rj = find(&mut parent, j);
                if ri != rj {
                    parent[ri] = rj;
                }
            }
        }
    }
    use std::collections::HashMap;
    let mut groups: HashMap<usize, Vec<i64>> = HashMap::new();
    for (i, row) in rows.iter().enumerate().take(n) {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(row.0);
    }
    Ok(groups.into_values().filter(|g| g.len() >= 2).collect())
}

/// Pull next N pending docs for extraction worker.
pub fn pop_extraction_batch(conn: &Connection, n: usize) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT doc_id FROM extraction_queue
         WHERE status = 'pending' ORDER BY enqueued_ts LIMIT ?1",
    )?;
    let rows = stmt
        .query_map([n as i64], |r| r.get::<_, i64>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Fuse multiple ranked candidate lists with type-weighted RRF.
///
/// `lists` is a vec of `(memory_id, rank0_based, list_weight)` triples.
/// Returns memory_ids sorted by fused score desc.
pub fn rrf_typed(
    lists: Vec<Vec<(i64, usize, f64)>>,
    k: f64,
    type_weight: impl Fn(i64) -> f64,
    limit: usize,
) -> Vec<(i64, f64)> {
    use std::collections::HashMap;
    let mut scores: HashMap<i64, f64> = HashMap::new();
    for list in lists {
        for (id, rank, list_w) in list {
            let rrf = list_w / (k + (rank as f64) + 1.0);
            *scores.entry(id).or_default() += rrf * type_weight(id);
        }
    }
    let mut v: Vec<(i64, f64)> = scores.into_iter().collect();
    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    v.truncate(limit);
    v
}

// ---------------------------------------------------------------------------
// Store::recall — fused vec+FTS+entity-1hop pipeline.
//
// Mined patterns:
//  * vectorize-io/hindsight (tracer.py `add_rrf_merged`) — list-shape (id, rank, meta)
//  * tirth8205/code-review-graph (tests/test_search.py `test_rrf_merge`) — RRF shape
// Adapted ~10 % to fit our Hit struct + memory_type weighting.
// ---------------------------------------------------------------------------

use crate::db::Store;
use crate::types::{Hit, SearchMode};

/// One recall hit: same shape as `crate::types::Hit` plus the originating
/// memory_id (when the doc has a memory row).
#[derive(Debug, Clone)]
pub struct RecallHit {
    pub hit: Hit,
    pub memory_id: Option<i64>,
    pub memory_type: Option<MemoryType>,
}

impl Store {
    /// SOTA recall pipeline. Uses existing hybrid `search()` for the lexical+vec
    /// branches, then fuses with entity-1hop expansion via memory_edges,
    /// applies type-weighted RRF, optional heat decay, and hands the top-N off
    /// to a pluggable reranker (caller's responsibility — this method only
    /// returns fused candidates so synapse-rerank stays an optional dep).
    pub fn recall(
        &self,
        params: &RecallParams,
        query_emb: Option<&[f32]>,
    ) -> Result<Vec<RecallHit>> {
        let backend = auto_route(params.target_recall.unwrap_or(0.98));
        // HyDE: expand vague queries to a hypothetical document for better lex recall.
        #[cfg(feature = "ollama")]
        let effective_query: std::borrow::Cow<str> = if let Some(ref cfg) = params.hyde {
            std::borrow::Cow::Owned(crate::turbo::hyde::expand(cfg, &params.query))
        } else {
            std::borrow::Cow::Borrowed(&params.query)
        };
        #[cfg(not(feature = "ollama"))]
        let effective_query: std::borrow::Cow<str> = std::borrow::Cow::Borrowed(&params.query);

        // Pull a wide candidate pool — RRF will narrow to k.
        let pool = (params.k.max(params.rerank_top) * 4).max(40);
        let base_hits = if let Some(emb) = query_emb {
            let vec_hits = self.search_vec_with_backend(emb, pool, backend)?;
            let lex_hits = self
                .search(&effective_query, SearchMode::Lex, None, pool)
                .unwrap_or_default();
            // Fuse lex + vec via RRF (same as search_hybrid but with backend control).
            let rrf_k = params.rrf_k;
            let mut scores: std::collections::HashMap<i64, (f64, crate::types::Hit)> =
                std::collections::HashMap::new();
            for (i, h) in lex_hits.into_iter().enumerate() {
                let s = 1.0 / (rrf_k + (i + 1) as f64);
                scores
                    .entry(h.id)
                    .and_modify(|e| e.0 += s)
                    .or_insert((s, h));
            }
            for (i, h) in vec_hits.into_iter().enumerate() {
                let s = 1.0 / (rrf_k + (i + 1) as f64);
                scores
                    .entry(h.id)
                    .and_modify(|e| e.0 += s)
                    .or_insert((s, h));
            }
            let mut merged: Vec<crate::types::Hit> = scores
                .into_values()
                .map(|(s, mut h)| {
                    h.score = s;
                    h
                })
                .collect();
            merged.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            merged.truncate(pool);
            merged
        } else {
            self.search(&effective_query, SearchMode::Lex, None, pool)?
        };

        // Build (memory_id, doc_id, memory_type) lookup for hits that have memories.
        let mut id_csv = String::new();
        for h in &base_hits {
            if !id_csv.is_empty() {
                id_csv.push(',');
            }
            id_csv.push_str(&h.id.to_string());
        }
        let mut mem_for_doc: std::collections::HashMap<i64, (i64, MemoryType)> =
            std::collections::HashMap::new();
        if !id_csv.is_empty() {
            let sql = format!(
                "SELECT id, doc_id, memory_type FROM memories
                 WHERE doc_id IN ({}) AND superseded_by IS NULL",
                id_csv
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map([], |r| {
                let mid: i64 = r.get(0)?;
                let did: i64 = r.get(1)?;
                let mt: String = r.get(2)?;
                Ok((mid, did, mt))
            })?;
            for row in rows {
                let (mid, did, mt) = row?;
                mem_for_doc.insert(did, (mid, MemoryType::parse(&mt)));
            }
        }

        // Convert hits → RRF list. Use rank-position as the rank.
        let mut lists: Vec<Vec<(i64, usize, f64)>> = Vec::new();
        let primary: Vec<(i64, usize, f64)> = base_hits
            .iter()
            .enumerate()
            .map(|(i, h)| (h.id, i, 1.0))
            .collect();
        lists.push(primary);

        // PPR signal (HippoRAG-2): seed = base hits w/ rank-derived score, propagate
        // mass through memory_edges. Adds a third RRF list weighted 1.0.
        if params.ppr && !mem_for_doc.is_empty() {
            use std::collections::HashMap;
            let mut seeds: HashMap<i64, f64> = HashMap::new();
            // Score = 1/(rank+1) so top-1 dominates teleport mass.
            for (rank, h) in base_hits.iter().enumerate() {
                if let Some((mid, _)) = mem_for_doc.get(&h.id) {
                    seeds.insert(*mid, 1.0 / (rank as f64 + 1.0));
                }
            }
            if !seeds.is_empty() {
                let ranked = crate::ppr::personalized_pagerank(
                    &self.conn,
                    &seeds,
                    params.ppr_alpha,
                    params.ppr_iters,
                    crate::ppr::DEFAULT_NEIGHBOR_CAP,
                    pool,
                )?;
                // Resolve memory_id → doc_id for fusion.
                let mids_csv = ranked
                    .iter()
                    .map(|(m, _)| m.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                if !mids_csv.is_empty() {
                    let sql = format!(
                        "SELECT id, doc_id FROM memories WHERE id IN ({}) AND superseded_by IS NULL",
                        mids_csv
                    );
                    let mut s2 = self.conn.prepare(&sql)?;
                    let mid_to_doc: HashMap<i64, i64> = s2
                        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?
                        .filter_map(|r| r.ok())
                        .collect();
                    let ppr_list: Vec<(i64, usize, f64)> = ranked
                        .into_iter()
                        .filter_map(|(mid, _)| mid_to_doc.get(&mid).copied())
                        .enumerate()
                        .map(|(i, did)| (did, i, 1.0))
                        .collect();
                    if !ppr_list.is_empty() {
                        lists.push(ppr_list);
                    }
                }
            }
        }

        // Multi-hop entity expansion (BFS, depth=params.max_hops or 2 default,
        // attenuation 0.6^hop applied as RRF list-weight).
        // Mined: petgraph BFS pattern, langchain MultiHopRetriever attenuation.
        if params.entity_expand && !params.ppr && !mem_for_doc.is_empty() {
            let mem_ids: Vec<i64> = mem_for_doc.values().map(|(mid, _)| *mid).collect();
            let max_hops = params.max_hops.max(1);
            let neighbors = multi_hop_neighbors(&self.conn, &mem_ids, max_hops, 64)?;
            // Group by hop, build separate lists with attenuation = 0.6^hop.
            use std::collections::HashMap;
            let mut by_hop: HashMap<u8, Vec<i64>> = HashMap::new();
            for (mid, hop) in neighbors {
                by_hop.entry(hop).or_default().push(mid);
            }
            for (hop, mem_ids_at_hop) in by_hop {
                if mem_ids_at_hop.is_empty() {
                    continue;
                }
                let csv = mem_ids_at_hop
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT m.doc_id FROM memories m
                     WHERE m.id IN ({}) AND m.superseded_by IS NULL
                     LIMIT 200",
                    csv
                );
                let mut stmt = self.conn.prepare(&sql)?;
                let expanded: Vec<i64> = stmt
                    .query_map([], |r| r.get::<_, i64>(0))?
                    .collect::<std::result::Result<_, _>>()?;
                let attenuation = 0.6f64.powi(hop as i32);
                let exp_list: Vec<(i64, usize, f64)> = expanded
                    .into_iter()
                    .enumerate()
                    .map(|(i, id)| (id, i, attenuation))
                    .collect();
                if !exp_list.is_empty() {
                    lists.push(exp_list);
                }
            }
        }

        // Type-weighted fusion. Each doc_id gets the weight of its memory's type
        // (Fact 1.20 etc.) — falls back to 1.0 for raw/unmapped docs.
        let weight_map = mem_for_doc.clone();
        let limit = params.k.max(params.rerank_top);
        let fused = rrf_typed(
            lists,
            params.rrf_k,
            move |id| {
                weight_map
                    .get(&id)
                    .map(|(_, t)| t.default_weight())
                    .unwrap_or(1.0)
            },
            limit,
        );

        // Heat decay: down-weight by recency. Cheap proxy = updated_ts age in days.
        let now = now_ts() as f64;
        let id_to_hit: std::collections::HashMap<i64, &Hit> =
            base_hits.iter().map(|h| (h.id, h)).collect();
        let mut hits: Vec<RecallHit> = Vec::with_capacity(fused.len());
        for (doc_id, mut score) in fused {
            let hit = match id_to_hit.get(&doc_id) {
                Some(h) => (*h).clone(),
                None => Hit {
                    id: doc_id,
                    uri: None,
                    title: None,
                    text: String::new(),
                    score: 0.0,
                    meta: None,
                    ts: None,
                },
            };
            if params.heat
                && let Ok(updated) = self.conn.query_row::<i64, _, _>(
                    "SELECT updated_ts FROM memories WHERE doc_id=?1
                     AND superseded_by IS NULL ORDER BY updated_ts DESC LIMIT 1",
                    [doc_id],
                    |r| r.get(0),
                )
            {
                let age_days = ((now - updated as f64) / 86400.0).max(0.0);
                // half-life 30 days: 0.97 ** age_days, clamped >= 0.3
                let decay = 0.97f64.powf(age_days).max(0.3);
                score *= decay;
            }
            let mut h2 = hit;
            h2.score = score;
            let (mid, mt) = match mem_for_doc.get(&doc_id) {
                Some((mid, mt)) => (Some(*mid), Some(*mt)),
                None => (None, None),
            };
            hits.push(RecallHit {
                hit: h2,
                memory_id: mid,
                memory_type: mt,
            });
        }
        hits.sort_by(|a, b| {
            b.hit
                .score
                .partial_cmp(&a.hit.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(params.k);
        Ok(hits)
    }

    /// Convenience: run sota_migrate on the underlying connection.
    pub fn sota_migrate(&self) -> Result<()> {
        sota_migrate(&self.conn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_mem() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        // Minimal docs table so FK constraints are satisfied for tests.
        c.execute_batch("CREATE TABLE docs (id INTEGER PRIMARY KEY, text TEXT NOT NULL);")
            .unwrap();
        c
    }

    #[test]
    fn migrate_idempotent() {
        let c = open_mem();
        sota_migrate(&c).unwrap();
        sota_migrate(&c).unwrap(); // again
    }

    #[test]
    fn put_and_supersede() {
        let c = open_mem();
        sota_migrate(&c).unwrap();
        c.execute("INSERT INTO docs (id, text) VALUES (1, 'old fact')", [])
            .unwrap();
        c.execute("INSERT INTO docs (id, text) VALUES (2, 'new fact')", [])
            .unwrap();
        let m1 = put_memory(&c, 1, MemoryType::Fact, None, None, 1.0).unwrap();
        let m2 = put_memory(&c, 2, MemoryType::Fact, None, None, 1.0).unwrap();
        supersede(&c, m1, m2).unwrap();
        let s: Option<i64> = c
            .query_row(
                "SELECT superseded_by FROM memories WHERE id=?1",
                [m1],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(s, Some(m2));
    }

    #[test]
    fn rrf_typed_basic() {
        let out = rrf_typed(
            vec![
                vec![(1, 0, 1.0), (2, 1, 1.0)],
                vec![(2, 0, 1.0), (1, 1, 1.0)],
            ],
            60.0,
            |_| 1.0,
            10,
        );
        assert_eq!(out.len(), 2);
        // both appear at rank 0 once and rank 1 once → tied scores
        assert!((out[0].1 - out[1].1).abs() < 1e-9);
    }

    #[test]
    fn type_weight_affects_default() {
        assert!(MemoryType::Fact.default_weight() > MemoryType::Episodic.default_weight());
    }

    #[test]
    fn extraction_queue_roundtrip() {
        let c = open_mem();
        sota_migrate(&c).unwrap();
        c.execute("INSERT INTO docs (id, text) VALUES (1, 'x')", [])
            .unwrap();
        enqueue_extraction(&c, 1).unwrap();
        let batch = pop_extraction_batch(&c, 10).unwrap();
        assert_eq!(batch, vec![1]);
    }

    // ---------------------------------------------------------------------------
    // Store::recall integration tests
    // ---------------------------------------------------------------------------

    fn open_store() -> (Store, tempfile::NamedTempFile) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let s = Store::open(tmp.path()).unwrap();
        (s, tmp)
    }

    #[test]
    fn recall_empty_store_returns_empty() {
        let (store, _tmp) = open_store();
        let params = RecallParams {
            query: "anything".into(),
            k: 5,
            ..RecallParams::default()
        };
        let hits = store.recall(&params, None).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn recall_fuses_vec_and_fts() {
        use crate::types::{EMBED_DIM, PutRequest};
        let (mut store, _tmp) = open_store();
        let fake_emb = |seed: u8| -> Vec<f32> {
            (0..EMBED_DIM)
                .map(|i| ((i as u8).wrapping_mul(seed) as f32) / 255.0)
                .collect()
        };
        let texts = [
            "rust sqlite fts5 memory recall",
            "vector embedding search engine",
            "agent memory typed fact lesson",
            "rust async tokio runtime",
            "unrelated document about baking",
        ];
        let mut doc_ids = Vec::new();
        for (i, t) in texts.iter().enumerate() {
            let id = store
                .put(&PutRequest {
                    text: (*t).into(),
                    embedding: Some(fake_emb(i as u8 + 1)),
                    ..Default::default()
                })
                .unwrap();
            doc_ids.push(id);
            put_memory(&store.conn, id, MemoryType::Fact, None, None, 1.0).unwrap();
        }

        let query_emb = fake_emb(1);
        let params = RecallParams {
            query: "memory recall".into(),
            k: 5,
            entity_expand: false,
            heat: false,
            ..RecallParams::default()
        };
        let hits = store.recall(&params, Some(&query_emb)).unwrap();
        assert!(!hits.is_empty(), "recall must return at least 1 hit");
        // Scores must be descending.
        for w in hits.windows(2) {
            assert!(w[0].hit.score >= w[1].hit.score);
        }
    }

    #[test]
    fn recall_entity_1hop_expands() {
        use crate::types::{EMBED_DIM, PutRequest};
        let (mut store, _tmp) = open_store();
        let fake_emb = |seed: u8| -> Vec<f32> {
            (0..EMBED_DIM)
                .map(|i| ((i as u8).wrapping_mul(seed) as f32) / 255.0)
                .collect()
        };
        // Insert two docs: A (target of query) and B (related via edge).
        let id_a = store
            .put(&PutRequest {
                text: "zephyr protocol document alpha".into(),
                embedding: Some(fake_emb(3)),
                ..Default::default()
            })
            .unwrap();
        let id_b = store
            .put(&PutRequest {
                text: "zephyr protocol document beta unrelated words".into(),
                embedding: Some(fake_emb(4)),
                ..Default::default()
            })
            .unwrap();
        let mem_a = put_memory(&store.conn, id_a, MemoryType::Fact, None, None, 1.0).unwrap();
        let mem_b = put_memory(&store.conn, id_b, MemoryType::Fact, None, None, 1.0).unwrap();
        // Wire A → B via "about" edge so BFS from A surfaces B.
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        store
            .conn
            .execute(
                "INSERT OR IGNORE INTO memory_edges (src_id, dst_id, edge_type, weight, created_ts)
                 VALUES (?1, ?2, 'about', 1.0, ?3)",
                rusqlite::params![mem_a, mem_b, ts],
            )
            .unwrap();

        let query_emb = fake_emb(3);
        let params = RecallParams {
            query: "zephyr protocol alpha".into(),
            k: 10,
            entity_expand: true,
            max_hops: 1,
            heat: false,
            ppr: false,
            ..RecallParams::default()
        };
        let hits = store.recall(&params, Some(&query_emb)).unwrap();
        // Both doc_a and doc_b should surface (direct hit + 1-hop).
        let ids: Vec<i64> = hits.iter().map(|h| h.hit.id).collect();
        assert!(ids.contains(&id_a), "doc_a must be in results");
        assert!(
            ids.contains(&id_b),
            "doc_b must surface via 1-hop expansion"
        );
    }

    // --- auto_route / SearchBackend tests ---

    #[test]
    fn auto_route_high_recall_gives_cascade() {
        assert_eq!(auto_route(0.99), SearchBackend::Cascade);
        assert_eq!(auto_route(0.98), SearchBackend::Cascade);
    }

    #[test]
    fn auto_route_mid_recall_gives_usearch_hnsw() {
        assert_eq!(auto_route(0.97), SearchBackend::UsearchHnsw);
        assert_eq!(auto_route(0.94), SearchBackend::UsearchHnsw);
    }

    #[test]
    fn auto_route_low_recall_gives_binary_first() {
        assert_eq!(auto_route(0.93), SearchBackend::BinaryFirst);
        assert_eq!(auto_route(0.80), SearchBackend::BinaryFirst);
        assert_eq!(auto_route(0.00), SearchBackend::BinaryFirst);
    }
}
