//! synapse-extract: async fact / entity / type extraction.
//!
//! Pipeline (async, off ingest critical path):
//!   docs.put → enqueue_extraction(doc_id)
//!   worker loop:
//!     pop_extraction_batch(N=8)
//!     → Extractor::extract(text) → Vec<ExtractedMemory>
//!     → upsert entities
//!     → put_memory(typed) per ExtractedMemory
//!     → mark queue row 'done'
//!
//! Default extractor is `RuleExtractor` — zero-deps regex heuristics for tests
//! and CI. Real workloads use `MlxExtractor` (smollm2-1.7B) under feature `mlx`.

use anyhow::Result;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use synapse_core::sota::{
    enqueue_extraction, pop_extraction_batch, put_memory, put_memory_with_date, MemoryType,
};

#[cfg(feature = "minimax")]
pub mod minimax;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedMemory {
    pub memory_type: MemoryType,
    pub entity: Option<String>,
    pub fact: String,
    pub confidence: f64,
    /// ISO 8601 / YYYY-MM-DD if extractor parsed an explicit event date.
    /// Distinct from `created_ts` (when the memory row was inserted).
    #[serde(default)]
    pub event_date: Option<String>,
    /// Subject-verb-object triples extracted from the text. Auto-relate hook
    /// converts these into `synapse_graph::edges` rows for traversal/PPR.
    /// Subject and object are upserted as entities; the edge weight defaults
    /// to the memory's `confidence` (clamped to [0.1, 1.0]).
    #[serde(default)]
    pub relations: Vec<ExtractedRelation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedRelation {
    pub subject: String,
    pub verb: String,
    pub object: String,
    /// Optional override for edge weight (else inherits memory confidence).
    #[serde(default)]
    pub weight: Option<f64>,
}

pub trait Extractor: Send + Sync {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedMemory>>;
    fn name(&self) -> &'static str;

    // ----- SOTA pipeline extensions (default no-op fallbacks) -----
    // Real implementations live in `MlxExtractor` (subprocess to local LLM).
    // Sources: ported from langchain/llamaindex/Self-RAG/HyDE Python references,
    // adapted to deterministic Rust fallbacks for CI without an LLM in the loop.

    /// Summarize a cluster of related memories into one canonical statement.
    /// Default: concatenate first 200 chars of each item.
    fn summarize(&self, items: &[&str]) -> Result<String> {
        let mut out = String::new();
        for s in items {
            if !out.is_empty() {
                out.push_str(" / ");
            }
            let head: String = s.chars().take(200).collect();
            out.push_str(head.trim());
        }
        Ok(out)
    }

    /// Merge new fact into existing memory text. Default: append if non-redundant.
    fn merge(&self, existing: &str, new_text: &str) -> Result<String> {
        if existing.contains(new_text.trim()) {
            return Ok(existing.to_string());
        }
        Ok(format!("{}\n{}", existing.trim_end(), new_text.trim()))
    }

    /// Decompose a multi-hop query into atomic sub-queries.
    /// Default: split on temporal/comparison cue words ("after", "before",
    /// "and then", "while", "vs"). Ported from langchain decomposition prompt.
    fn decompose_query(&self, query: &str) -> Result<Vec<String>> {
        let cues = [" after ", " before ", " and then ", " while ", " vs ", " versus "];
        let lower = query.to_lowercase();
        for cue in cues.iter() {
            if let Some(pos) = lower.find(cue) {
                let cue_len = cue.len();
                let a = query[..pos].trim().to_string();
                let b = query[pos + cue_len..].trim().to_string();
                if !a.is_empty() && !b.is_empty() {
                    return Ok(vec![a, b]);
                }
            }
        }
        Ok(vec![query.to_string()])
    }

    /// Self-RAG style relevance grade (0.0..=1.0). Default: token-overlap fallback.
    fn grade_relevance(&self, query: &str, doc: &str) -> Result<f64> {
        let q_tokens: std::collections::HashSet<_> = query
            .to_lowercase()
            .split_whitespace()
            .filter(|t| t.len() > 2)
            .map(|s| s.to_string())
            .collect();
        if q_tokens.is_empty() {
            return Ok(0.5);
        }
        let d_lower = doc.to_lowercase();
        let hits = q_tokens.iter().filter(|t| d_lower.contains(t.as_str())).count();
        Ok((hits as f64 / q_tokens.len() as f64).min(1.0))
    }

    /// HyDE: Hypothetical Document Embeddings.
    /// Generate a plausible answer to seed a richer embedding query.
    /// Default: echo + simple expansion ("Answer: <q>. The answer involves <q>.").
    /// Ported from Gao et al. 2022 HyDE prompt template.
    fn hyde(&self, query: &str) -> Result<String> {
        Ok(format!(
            "The answer to '{q}' is as follows. {q} relates to specific facts and recent context.",
            q = query.trim()
        ))
    }
}

/// Cheap deterministic extractor — recognizes simple patterns. CI / fallback.
pub struct RuleExtractor;

impl Extractor for RuleExtractor {
    fn name(&self) -> &'static str {
        "rule"
    }

    fn extract(&self, text: &str) -> Result<Vec<ExtractedMemory>> {
        let lower = text.to_lowercase();
        let mut out = Vec::new();

        // Very small heuristic set — placeholder until MLX backend is wired.
        if lower.contains("decided") || lower.contains("chose") || lower.contains("we will") {
            out.push(ExtractedMemory {
                memory_type: MemoryType::Decision,
                entity: None,
                fact: text.trim().to_string(),
                confidence: 0.6,
                event_date: None,
                relations: Vec::new(),
            });
        } else if lower.contains("learned")
            || lower.contains("turns out")
            || lower.contains("lesson")
        {
            out.push(ExtractedMemory {
                memory_type: MemoryType::Lesson,
                entity: None,
                fact: text.trim().to_string(),
                confidence: 0.6,
                event_date: None,
                relations: Vec::new(),
            });
        } else if lower.contains("prefer") || lower.contains("likes") || lower.contains("hates") {
            out.push(ExtractedMemory {
                memory_type: MemoryType::Preference,
                entity: None,
                fact: text.trim().to_string(),
                confidence: 0.55,
                event_date: None,
                relations: Vec::new(),
            });
        } else {
            out.push(ExtractedMemory {
                memory_type: MemoryType::Fact,
                entity: None,
                fact: text.trim().to_string(),
                confidence: 0.5,
                event_date: None,
                relations: Vec::new(),
            });
        }
        Ok(out)
    }
}

/// MLX subprocess extractor — calls out to `mlx_lm.generate` with smollm2.
/// Stub: returns rule-extractor output until subprocess wiring is finalized.
#[cfg(feature = "mlx")]
pub struct MlxExtractor {
    pub model: String,
    pub max_tokens: u32,
}

#[cfg(feature = "mlx")]
impl Default for MlxExtractor {
    fn default() -> Self {
        Self {
            model: "mlx-community/SmolLM2-1.7B-Instruct-4bit".into(),
            max_tokens: 256,
        }
    }
}

#[cfg(feature = "mlx")]
impl Extractor for MlxExtractor {
    fn name(&self) -> &'static str {
        "mlx-smollm2"
    }
    fn extract(&self, text: &str) -> Result<Vec<ExtractedMemory>> {
        // TODO: spawn mlx_lm.generate via std::process::Command, parse JSON.
        // Until then, fall back to rule extractor so dev builds keep working.
        RuleExtractor.extract(text)
    }
}

/// Auto-relate hook: convert ExtractedRelation triples into edges.
///
/// For each (s,v,o) triple, upserts subject + object as entities, then writes
/// a row into `synapse_graph::edges`. Edge weight defaults to memory confidence,
/// clamped to [0.1, 1.0]. Idempotent: INSERT OR REPLACE keyed on (from,to,rel).
///
/// Failure to ensure edges schema or insert is logged but does NOT fail the
/// surrounding extraction — graph layer is best-effort overlay on memories.
pub fn relate_extracted(
    conn: &Connection,
    relations: &[ExtractedRelation],
    fallback_confidence: f64,
) -> Result<usize> {
    if relations.is_empty() {
        return Ok(0);
    }
    if let Err(e) = synapse_graph::ensure_schema(conn) {
        tracing::warn!("graph schema ensure failed (skipping auto-relate): {e}");
        return Ok(0);
    }
    let mut count = 0usize;
    for r in relations {
        let s_id = match upsert_entity(conn, r.subject.trim(), None) {
            Ok(i) => i,
            Err(e) => { tracing::warn!("upsert subject failed: {e}"); continue; }
        };
        let o_id = match upsert_entity(conn, r.object.trim(), None) {
            Ok(i) => i,
            Err(e) => { tracing::warn!("upsert object failed: {e}"); continue; }
        };
        let w = r.weight.unwrap_or(fallback_confidence).clamp(0.1, 1.0);
        match synapse_graph::relate(conn, s_id, o_id, r.verb.trim(), w, None) {
            Ok(_) => count += 1,
            Err(e) => tracing::warn!("relate failed: {e}"),
        }
    }
    Ok(count)
}

/// Find-or-create entity by canonical name. Returns its id.
pub fn upsert_entity(conn: &Connection, canonical: &str, etype: Option<&str>) -> Result<i64> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    if let Some(id) = conn
        .query_row(
            "SELECT id FROM entities WHERE canonical_name = ?1",
            [canonical],
            |r| r.get::<_, i64>(0),
        )
        .ok()
    {
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO entities (canonical_name, entity_type, created_ts) VALUES (?1, ?2, ?3)",
        rusqlite::params![canonical, etype, now],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Run one extraction pass over the queue. Returns count of memories produced.
pub fn run_once(conn: &Connection, extractor: &dyn Extractor, batch: usize) -> Result<usize> {
    let doc_ids = pop_extraction_batch(conn, batch)?;
    let mut produced = 0usize;
    for doc_id in doc_ids {
        let text: String = conn
            .query_row("SELECT text FROM docs WHERE id = ?1", [doc_id], |r| {
                r.get(0)
            })
            .unwrap_or_default();
        if text.is_empty() {
            mark_status(conn, doc_id, "skipped", Some("empty text"))?;
            continue;
        }
        match extractor.extract(&text) {
            Ok(items) => {
                for it in items {
                    let entity_id = match &it.entity {
                        Some(name) => Some(upsert_entity(conn, name, None)?),
                        None => None,
                    };
                    if it.event_date.is_some() {
                        put_memory_with_date(
                            conn,
                            doc_id,
                            it.memory_type,
                            entity_id,
                            None,
                            it.confidence,
                            it.event_date.as_deref(),
                        )?;
                    } else {
                        put_memory(
                            conn,
                            doc_id,
                            it.memory_type,
                            entity_id,
                            None,
                            it.confidence,
                        )?;
                    }
                    // Auto-relate: write extracted triples into edges.
                    let _ = relate_extracted(conn, &it.relations, it.confidence);
                    produced += 1;
                }
                mark_status(conn, doc_id, "done", None)?;
            }
            Err(e) => {
                mark_status(conn, doc_id, "error", Some(&e.to_string()))?;
            }
        }
    }
    Ok(produced)
}

fn mark_status(conn: &Connection, doc_id: i64, status: &str, err: Option<&str>) -> Result<()> {
    conn.execute(
        "UPDATE extraction_queue SET status=?1, attempts=attempts+1, last_error=?2 WHERE doc_id=?3",
        rusqlite::params![status, err, doc_id],
    )?;
    Ok(())
}

/// Write pre-extracted memories to a `Connection`. Used by callers that ran
/// extraction off-thread (parallel HTTP) and now want to persist results.
/// Up-serts entities + uses `put_memory_with_date` when an event_date exists.
pub fn enqueue_extraction_helper(
    conn: &Connection,
    doc_id: i64,
    items: &[ExtractedMemory],
) -> Result<usize> {
    let mut produced = 0usize;
    for it in items {
        let entity_id = match &it.entity {
            Some(name) => Some(upsert_entity(conn, name, None)?),
            None => None,
        };
        if it.event_date.is_some() {
            put_memory_with_date(
                conn,
                doc_id,
                it.memory_type,
                entity_id,
                None,
                it.confidence,
                it.event_date.as_deref(),
            )?;
        } else {
            put_memory(conn, doc_id, it.memory_type, entity_id, None, it.confidence)?;
        }
        // Auto-relate: write extracted triples into edges.
        let _ = relate_extracted(conn, &it.relations, it.confidence);
        produced += 1;
    }
    Ok(produced)
}

/// Convenience: enqueue + run synchronously (for tests / small batches).
pub fn ingest_and_extract(
    conn: &Connection,
    doc_id: i64,
    extractor: &dyn Extractor,
) -> Result<usize> {
    enqueue_extraction(conn, doc_id)?;
    run_once(conn, extractor, 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use synapse_core::sota::sota_migrate;

    fn fresh() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE docs (id INTEGER PRIMARY KEY, text TEXT NOT NULL);")
            .unwrap();
        sota_migrate(&c).unwrap();
        c
    }

    #[test]
    fn rule_classifies_decision() {
        let r = RuleExtractor;
        let out = r.extract("We decided to use SQLite").unwrap();
        assert_eq!(out[0].memory_type, MemoryType::Decision);
    }

    #[test]
    fn rule_classifies_lesson() {
        let r = RuleExtractor;
        let out = r.extract("Lesson: never block tokio runtime").unwrap();
        assert_eq!(out[0].memory_type, MemoryType::Lesson);
    }

    #[test]
    fn end_to_end_queue_flow() {
        let c = fresh();
        c.execute(
            "INSERT INTO docs (id, text) VALUES (1, 'We chose Rust over Go')",
            [],
        )
        .unwrap();
        let n = ingest_and_extract(&c, 1, &RuleExtractor).unwrap();
        assert!(n >= 1);
        let count: i64 = c
            .query_row("SELECT COUNT(*) FROM memories WHERE memory_type='decision'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(count >= 1);
    }

    #[test]
    fn upsert_entity_idempotent() {
        let c = fresh();
        let a = upsert_entity(&c, "Alice", Some("person")).unwrap();
        let b = upsert_entity(&c, "Alice", Some("person")).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn auto_relate_writes_edges() {
        let c = fresh();
        let rels = vec![
            ExtractedRelation {
                subject: "Alice".into(),
                verb: "works_at".into(),
                object: "Acme".into(),
                weight: None,
            },
            ExtractedRelation {
                subject: "Acme".into(),
                verb: "located_in".into(),
                object: "Berlin".into(),
                weight: Some(0.9),
            },
        ];
        let n = relate_extracted(&c, &rels, 0.7).unwrap();
        assert_eq!(n, 2);
        let edges: i64 = c.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0)).unwrap();
        assert_eq!(edges, 2);
        let weight_acme: f64 = c.query_row(
            "SELECT weight FROM edges WHERE rel='located_in'", [], |r| r.get(0)).unwrap();
        assert!((weight_acme - 0.9).abs() < 1e-9);
    }

    #[test]
    fn auto_relate_clamps_weight() {
        let c = fresh();
        let rels = vec![ExtractedRelation {
            subject: "X".into(), verb: "rel".into(), object: "Y".into(),
            weight: Some(2.5),
        }];
        relate_extracted(&c, &rels, 0.5).unwrap();
        let w: f64 = c.query_row("SELECT weight FROM edges", [], |r| r.get(0)).unwrap();
        assert!(w <= 1.0 && w >= 0.1);
    }
}
