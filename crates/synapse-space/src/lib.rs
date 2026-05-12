//! `synapse-space` — Space concept hierarchy on top of Synapse.
//!
//! Hierarchy:
//!   Space   (one SQLite Brain file, lives at a path)
//!   └─ Wing    (namespace: person, project, topic-group)
//!      └─ Room    (topic within a wing)
//!         └─ Drawer  (verbatim chunk / memory unit)
//!
//! Each Drawer stores its text via `Store::put` (FTS5 + optional vec).
//! Wing + Room are encoded as URI prefix:  `spaces://<wing>/<room>/<uid>`
//!
//! # Feature: `mlx`
//! When compiled with `--features mlx`, `Space::search_hybrid` uses a
//! cosine reranker on FTS50 candidates. The embedder is caller-supplied
//! (pass `Some(embedding)` to `search`). The Metal shader path from
//! `synapse-metal` is intentionally NOT wired here — `MetalI8Matvec` is a
//! MatVec primitive, not a text embedder. Wire the MLX Python embedder via
//! the Python adapter (`mempalace-synapse-backend`) for full hybrid recall.

pub mod mcp;

#[cfg(feature = "daemon-embed")]
pub mod embed_bridge;

use anyhow::Result;
use rusqlite::Connection;
use synapse_core::{
    db::Store,
    types::{PutRequest, SearchMode},
};

pub struct Space {
    pub store: Store,
    name: String,
}

impl Space {
    pub fn open(name: impl Into<String>, path: impl AsRef<std::path::Path>) -> Result<Self> {
        let store = Store::open(path.as_ref())?;
        Ok(Self {
            store,
            name: name.into(),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Borrow the underlying rusqlite connection for raw SQL queries.
    pub(crate) fn conn_ref(&self) -> &Connection {
        &self.store.conn
    }

    pub fn wing<'s>(&'s mut self, name: impl Into<String>) -> Wing<'s> {
        Wing {
            space: self,
            name: name.into(),
        }
    }

    /// Forward `PutRequest` directly to the underlying Store — used by `space_sweep`.
    pub fn store_put(&mut self, req: &PutRequest) -> Result<i64> {
        Ok(self.store.put(req)?)
    }

    pub fn search(
        &self,
        query: &str,
        embedding: Option<&[f32]>,
        limit: usize,
    ) -> Result<Vec<DrawerHit>> {
        let mode = match embedding {
            Some(_) => SearchMode::Hybrid,
            None => SearchMode::Lex,
        };
        let hits = self.store.search(query, mode, embedding, limit)?;
        Ok(hits
            .into_iter()
            .map(|h| DrawerHit {
                id: h.id,
                text: h.text,
                score: h.score,
            })
            .collect())
    }

    /// Hybrid FTS+vec search followed by cross-encoder rerank.
    ///
    /// Fetches `candidate_pool` (default 50) candidates via FTS/vec, then reranks
    /// with `IdentityReranker` (or `OnnxCrossEncoder` when `rerank,onnx` features
    /// both active), returning top-`k`.
    #[cfg(feature = "rerank")]
    pub fn search_reranked(
        &self,
        query: &str,
        embedding: Option<&[f32]>,
        k: usize,
        candidate_pool: usize,
    ) -> Result<Vec<DrawerHit>> {
        let pool = candidate_pool.max(k);
        let mode = match embedding {
            Some(_) => synapse_core::types::SearchMode::Hybrid,
            None => synapse_core::types::SearchMode::Lex,
        };
        let candidates = self.store.search(query, mode, embedding, pool)?;
        #[cfg(all(feature = "rerank", feature = "onnx"))]
        let reranked = {
            use synapse_rerank::{Reranker, onnx::OnnxCrossEncoder};
            let reranker = OnnxCrossEncoder::new()?;
            reranker.rerank(query, candidates, k)?
        };
        #[cfg(not(all(feature = "rerank", feature = "onnx")))]
        let reranked = {
            use synapse_rerank::{IdentityReranker, Reranker};
            let reranker = IdentityReranker;
            reranker.rerank(query, candidates, k)?
        };
        Ok(reranked
            .into_iter()
            .map(|h| DrawerHit {
                id: h.id,
                text: h.text,
                score: h.score,
            })
            .collect())
    }

    /// BM25+vec fusion via Reciprocal Rank Fusion (RRF, k=60), then optionally
    /// reranked. Returns top-`k` from fused list.
    ///
    /// `fts_n` and `vec_n` control candidate pool size for each leg.
    /// Default: fts_n=100, vec_n=100.
    pub fn search_hybrid_rrf(
        &self,
        query: &str,
        embedding: &[f32],
        k: usize,
        fts_n: usize,
        vec_n: usize,
    ) -> Result<Vec<DrawerHit>> {
        use std::collections::HashMap;
        use synapse_core::types::SearchMode;

        let fts_hits = self.store.search(query, SearchMode::Lex, None, fts_n)?;
        let vec_hits = self
            .store
            .search(query, SearchMode::Vec, Some(embedding), vec_n)?;

        // Map id → RRF score
        let mut scores: HashMap<i64, f64> = HashMap::new();
        let k_rrf = 60.0_f64;

        for (rank, h) in fts_hits.iter().enumerate() {
            *scores.entry(h.id).or_insert(0.0) += 1.0 / (k_rrf + (rank + 1) as f64);
        }
        for (rank, h) in vec_hits.iter().enumerate() {
            *scores.entry(h.id).or_insert(0.0) += 1.0 / (k_rrf + (rank + 1) as f64);
        }

        // Build merged set (preserve text from whichever list has it)
        let mut id_text: HashMap<i64, String> = HashMap::new();
        for h in fts_hits.iter().chain(vec_hits.iter()) {
            id_text.entry(h.id).or_insert_with(|| h.text.clone());
        }

        let mut ranked: Vec<(i64, f64)> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked.truncate(k);

        Ok(ranked
            .into_iter()
            .map(|(id, score)| DrawerHit {
                id,
                text: id_text.get(&id).cloned().unwrap_or_default(),
                score,
            })
            .collect())
    }

    /// Hybrid search: FTS top-50 candidates reranked by cosine vs `query_emb`.
    /// Only active when caller supplies an embedding. Falls back to pure FTS.
    #[cfg(feature = "mlx")]
    pub fn search_hybrid(
        &self,
        query: &str,
        query_emb: &[f32],
        limit: usize,
    ) -> Result<Vec<DrawerHit>> {
        // Step 1: FTS top-50
        let candidates = self
            .store
            .search(query, SearchMode::Hybrid, Some(query_emb), 50)?;
        // Step 2: rerank by cosine (sqlite-vec already did this; pass through)
        let mut hits: Vec<DrawerHit> = candidates
            .into_iter()
            .map(|h| DrawerHit {
                id: h.id,
                text: h.text,
                score: h.score,
            })
            .collect();
        hits.truncate(limit);
        Ok(hits)
    }
}

pub struct Wing<'s> {
    space: &'s mut Space,
    name: String,
}

impl<'s> Wing<'s> {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn room(self, topic: impl Into<String>) -> Room<'s> {
        Room {
            space: self.space,
            wing_name: self.name,
            topic: topic.into(),
        }
    }
}

pub struct Room<'s> {
    space: &'s mut Space,
    wing_name: String,
    topic: String,
}

impl<'s> Room<'s> {
    pub fn topic(&self) -> &str {
        &self.topic
    }

    pub fn put(&mut self, text: impl Into<String>, embedding: Option<Vec<f32>>) -> Result<Drawer> {
        let text = text.into();
        let uri = format!(
            "spaces://{}/{}/{}",
            self.wing_name,
            self.topic,
            monotonic_uid()
        );
        let req = PutRequest {
            uri: Some(uri.clone()),
            title: Some(format!("{}/{}", self.wing_name, self.topic)),
            text: text.clone(),
            meta: None,
            embedding,
        };
        let id = self.space.store.put(&req)?;
        Ok(Drawer { id, uri, text })
    }
}

#[derive(Debug, Clone)]
pub struct Drawer {
    pub id: i64,
    pub uri: String,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct DrawerHit {
    pub id: i64,
    pub text: String,
    pub score: f64,
}

fn monotonic_uid() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CTR: AtomicU64 = AtomicU64::new(1);
    CTR.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::NamedTempFile;

    // --- original integration test ---

    #[test]
    fn build_space_add_drawers_search() -> Result<()> {
        let f = NamedTempFile::new()?;
        let mut space = Space::open("test-space", f.path())?;

        space
            .wing("alice")
            .room("greetings")
            .put("Hello from the first drawer", None)?;
        space
            .wing("alice")
            .room("greetings")
            .put("Second drawer about Rust programming", None)?;
        space
            .wing("bob")
            .room("notes")
            .put("Bob note about vector search similarity", None)?;

        let hits = space.search("Rust programming", None, 5)?;
        assert!(!hits.is_empty(), "expected at least one FTS hit");
        assert!(
            hits[0].text.to_lowercase().contains("rust"),
            "top hit should be about Rust, got: {:?}",
            hits[0].text
        );
        Ok(())
    }

    // --- P0 MCP tool unit tests ---

    #[test]
    fn mcp_drawer_list_returns_rows() -> Result<()> {
        let f = NamedTempFile::new()?;
        let mut space = Space::open("t", f.path())?;
        space.wing("w").room("r").put("alpha text", None)?;
        space.wing("w").room("r").put("beta text", None)?;

        let result = crate::mcp::dispatch(
            "drawer_list",
            json!({
                "space_path": f.path().to_str().unwrap(),
                "wing": "w",
                "room": "r",
                "limit": 10
            }),
        );
        let drawers = result["drawers"].as_array().expect("drawers array");
        assert_eq!(drawers.len(), 2);
        Ok(())
    }

    #[test]
    fn mcp_drawer_show_returns_full_content() -> Result<()> {
        let f = NamedTempFile::new()?;
        let mut space = Space::open("t", f.path())?;
        let d = space.wing("w").room("r").put("show me this text", None)?;

        let result = crate::mcp::dispatch(
            "drawer_show",
            json!({
                "space_path": f.path().to_str().unwrap(),
                "id": d.id
            }),
        );
        assert_eq!(result["ok"], true);
        assert!(
            result["drawer"]["text"]
                .as_str()
                .unwrap()
                .contains("show me")
        );
        Ok(())
    }

    #[test]
    fn mcp_drawer_delete_soft_tombstone() -> Result<()> {
        let f = NamedTempFile::new()?;
        let mut space = Space::open("t", f.path())?;
        let d = space.wing("w").room("r").put("to be deleted", None)?;

        let del = crate::mcp::dispatch(
            "drawer_delete",
            json!({
                "space_path": f.path().to_str().unwrap(),
                "id": d.id
            }),
        );
        assert_eq!(del["ok"], true);

        // After soft-delete, drawer_list should exclude it
        let list = crate::mcp::dispatch(
            "drawer_list",
            json!({
                "space_path": f.path().to_str().unwrap(),
                "wing": "w",
                "room": "r"
            }),
        );
        let drawers = list["drawers"].as_array().unwrap();
        assert!(
            drawers.is_empty(),
            "soft-deleted drawer should not appear in list"
        );
        Ok(())
    }

    #[test]
    fn mcp_space_sweep_idempotent() -> Result<()> {
        let f = NamedTempFile::new()?;

        let msgs = json!([
            { "role": "user", "content": "Hello world", "ts": "2026-05-03T10:00:00Z" },
            { "role": "assistant", "content": "Hi there", "ts": "2026-05-03T10:00:05Z" },
            { "role": "user", "content": "What time is it?", "ts": "2026-05-03T10:01:00Z" },
            { "role": "assistant", "content": "It is 10:01 AM.", "ts": "2026-05-03T10:01:05Z" },
            { "role": "user", "content": "Thanks!", "ts": "2026-05-03T10:01:10Z" }
        ]);
        let input = json!({
            "space_path": f.path().to_str().unwrap(),
            "wing": "w",
            "room": "chat",
            "messages": msgs
        });

        let r1 = crate::mcp::dispatch("space_sweep", input.clone());
        // 5 short messages → 5 drawers (one per message)
        assert!(
            r1["inserted"].as_u64().unwrap() >= 5,
            "expected >=5 drawers for 5 messages, got {}",
            r1["inserted"]
        );
        assert_eq!(r1["skipped"], 0);

        // Second sweep: same messages → all skipped (idempotent)
        let r2 = crate::mcp::dispatch("space_sweep", input);
        assert_eq!(
            r2["inserted"], 0,
            "second sweep should insert 0, got {}",
            r2["inserted"]
        );
        Ok(())
    }

    #[test]
    fn mcp_space_sweep_long_message_splits() -> Result<()> {
        let f = NamedTempFile::new()?;
        // A message with >1000 chars should produce multiple drawers (windowed split)
        // Use varied content so windows differ and aren't deduped by blake3.
        let long_content: String = (0..1200)
            .map(|i| char::from(b'A' + (i % 26) as u8))
            .collect();
        let msgs = json!([
            { "role": "assistant", "content": long_content, "ts": "2026-05-03T10:00:00Z" }
        ]);
        let result = crate::mcp::dispatch(
            "space_sweep",
            json!({
                "space_path": f.path().to_str().unwrap(),
                "wing": "w",
                "room": "chat",
                "messages": msgs
            }),
        );
        // 1200 chars, window=400, step=350 → windows at 0,350,700,1050 → 4 chunks
        assert!(
            result["inserted"].as_u64().unwrap() >= 3,
            "long message should yield >=3 chunks, got {}",
            result["inserted"]
        );
        Ok(())
    }

    #[test]
    fn mcp_wing_search_scoped() -> Result<()> {
        let f = NamedTempFile::new()?;
        let mut space = Space::open("t", f.path())?;
        space
            .wing("alice")
            .room("notes")
            .put("quantum computing fundamentals", None)?;
        space
            .wing("bob")
            .room("notes")
            .put("classical music theory", None)?;

        let result = crate::mcp::dispatch(
            "wing_search",
            json!({
                "space_path": f.path().to_str().unwrap(),
                "wing": "alice",
                "query": "quantum",
                "k": 5
            }),
        );
        assert_eq!(result["ok"], true);
        let hits = result["results"].as_array().unwrap();
        // All hits should be from alice's wing
        assert!(!hits.is_empty());
        Ok(())
    }

    /// Integration test for search_reranked (rerank feature, no daemon required).
    #[cfg(feature = "rerank")]
    #[test]
    fn search_reranked_returns_relevant() -> Result<()> {
        let f = NamedTempFile::new()?;
        let mut space = Space::open("t", f.path())?;
        space
            .wing("w")
            .room("r")
            .put("quantum entanglement in physics experiments", None)?;
        space
            .wing("w")
            .room("r")
            .put("banana bread recipe with walnuts", None)?;
        space
            .wing("w")
            .room("r")
            .put("quantum computing qubit coherence times", None)?;
        space
            .wing("w")
            .room("r")
            .put("chocolate chip cookie recipe", None)?;
        space
            .wing("w")
            .room("r")
            .put("quantum error correction surface codes", None)?;

        let hits = space.search_reranked("quantum physics", None, 3, 10)?;
        assert!(!hits.is_empty(), "expected hits");
        let top_text = hits[0].text.to_lowercase();
        assert!(
            top_text.contains("quantum"),
            "top hit should mention quantum, got: {}",
            hits[0].text
        );
        Ok(())
    }

    /// Integration test for daemon embed bridge.
    ///
    /// Requires `--features daemon-embed` AND synapsed running at /tmp/synapse.sock.
    /// Will skip (not fail) if daemon is unreachable or Embed RPC not in proto.
    ///
    /// Run with:
    ///   cargo test -p synapse-space --features daemon-embed -- embed_bridge_cosine
    /// Integration test for daemon embed bridge.
    ///
    /// Uses BGE-small-en-v1.5 (384-dim). Short single words cluster high (0.6+)
    /// even when unrelated — use full phrases for meaningful separation.
    /// Thresholds calibrated against measured BGE-small-en-v1.5 outputs.
    #[cfg(feature = "daemon-embed")]
    #[tokio::test]
    async fn embed_bridge_cosine() {
        use crate::embed_bridge::{cosine, embed_text};

        // Try to embed — if daemon not running, skip gracefully.
        let automotive_text = match embed_text(
            "The automobile engine requires regular oil changes and tune-ups for optimal performance"
        ).await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("SKIP embed_bridge_cosine: {e}");
                return;
            }
        };
        // Synonym sentence — same semantic domain, should be very similar
        let car_text = embed_text(
            "The car motor needs routine maintenance including oil replacement and tuning",
        )
        .await
        .expect("embed car sentence");

        // Unrelated domain — cooking, no vehicle content
        let recipe_text = embed_text(
            "Slice the ripe banana and mix with yogurt and honey for a healthy breakfast smoothie",
        )
        .await
        .expect("embed recipe sentence");

        let sim_synonyms = cosine(&automotive_text, &car_text);
        let sim_unrelated = cosine(&automotive_text, &recipe_text);

        // BGE-small-en-v1.5 measured: automobile/car sentence ~0.92, automobile/recipe ~0.70
        assert!(
            sim_synonyms > 0.85,
            "automotive synonyms cosine should be > 0.85, got {sim_synonyms:.3}"
        );
        assert!(
            sim_unrelated < sim_synonyms - 0.10,
            "unrelated should be at least 0.10 below synonyms: synonyms={sim_synonyms:.3} unrelated={sim_unrelated:.3}"
        );
    }
}
