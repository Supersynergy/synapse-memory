//! ColBERT multi-vector SQLite storage + reranker.
//! Table: colbert_vecs(doc_id INTEGER, vecs BLOB)
//! vecs blob = zstd( json: Vec<Vec<f32>> )
//! Table: colbert_vecs_i8(doc_id INTEGER, vecs_i8 BLOB, scales BLOB)
//! vecs_i8 blob = raw i8 bytes (n_tokens × dim), scales blob = n_tokens × f32-le
//! Table: colbert_muvera(doc_id INTEGER, fde BLOB) [optional, muvera feature]
//! fde blob = raw f32-le bytes of FDE vector

use anyhow::Result;
use rusqlite::{Connection, params};
use crate::{ColbertEmbedder, max_sim};
use crate::quant::{quant_i8, max_sim_i8};

#[cfg(feature = "muvera")]
use crate::muvera::{muvera_encode, cosine_sim};

/// Config for MUVERA two-tier search.
#[cfg(feature = "muvera")]
#[derive(Clone, Debug)]
pub struct MuveraConfig {
    /// FDE output dimension; must be a multiple of ColBERT token dim (128).
    pub fde_dim: usize,
    /// RNG seed for reproducible hyperplane matrix.
    pub seed: u64,
}

#[cfg(feature = "muvera")]
impl Default for MuveraConfig {
    fn default() -> Self {
        Self { fde_dim: 1024, seed: 42 }
    }
}

pub struct ColbertStore<'a> {
    conn: &'a Connection,
    emb: ColbertEmbedder,
    #[cfg(feature = "muvera")]
    muvera_cfg: Option<MuveraConfig>,
}

impl<'a> ColbertStore<'a> {
    pub fn new(conn: &'a Connection) -> Result<Self> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS colbert_vecs (
                doc_id  INTEGER PRIMARY KEY,
                vecs    BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS colbert_vecs_i8 (
                doc_id  INTEGER PRIMARY KEY,
                vecs_i8 BLOB NOT NULL,
                scales  BLOB NOT NULL
            );"
        )?;
        Ok(Self {
            conn,
            emb: ColbertEmbedder::default(),
            #[cfg(feature = "muvera")]
            muvera_cfg: None,
        })
    }

    /// Create store with MUVERA FDE support enabled.
    #[cfg(feature = "muvera")]
    pub fn new_with_muvera(conn: &'a Connection, cfg: MuveraConfig) -> Result<Self> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS colbert_vecs (
                doc_id  INTEGER PRIMARY KEY,
                vecs    BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS colbert_vecs_i8 (
                doc_id  INTEGER PRIMARY KEY,
                vecs_i8 BLOB NOT NULL,
                scales  BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS colbert_muvera (
                doc_id  INTEGER PRIMARY KEY,
                fde     BLOB NOT NULL
            );"
        )?;
        Ok(Self {
            conn,
            emb: ColbertEmbedder::default(),
            muvera_cfg: Some(cfg),
        })
    }

    /// Store FDE for a doc (muvera feature). Silently no-ops if muvera not configured.
    #[cfg(feature = "muvera")]
    fn store_fde(&self, doc_id: i64, token_vecs: &[Vec<f32>]) -> Result<()> {
        let cfg = match &self.muvera_cfg {
            Some(c) => c,
            None => return Ok(()),
        };
        let fde = muvera_encode(token_vecs, cfg.fde_dim, cfg.seed);
        let fde_bytes: Vec<u8> = fde.iter().flat_map(|x| x.to_le_bytes()).collect();
        self.conn.execute(
            "INSERT OR REPLACE INTO colbert_muvera (doc_id, fde) VALUES (?1, ?2)",
            params![doc_id, fde_bytes],
        )?;
        Ok(())
    }

    /// Load FDE vector for a doc.
    #[cfg(feature = "muvera")]
    #[allow(dead_code)]
    fn load_fde(&self, doc_id: i64) -> Result<Vec<f32>> {
        let bytes: Vec<u8> = self.conn.query_row(
            "SELECT fde FROM colbert_muvera WHERE doc_id = ?1",
            params![doc_id],
            |row| row.get(0),
        )?;
        Ok(bytes.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect())
    }

    /// Two-tier MUVERA search: FDE-ANN (exhaustive dot-product) → ColBERT max-sim rerank.
    ///
    /// 1. Encode query token vecs → FDE
    /// 2. Dot-product scan all stored FDEs → top-N candidates
    /// 3. Load token vecs for top-N, ColBERT max-sim rerank
    /// 4. Return top-K (doc_id, score) sorted desc
    #[cfg(feature = "muvera")]
    pub fn search_muvera_then_rerank(
        &self,
        query_token_vecs: &[Vec<f32>],
        k: usize,
        fde_top_n: usize,
    ) -> Result<Vec<(i64, f32)>> {
        let cfg = self.muvera_cfg.as_ref()
            .ok_or_else(|| anyhow::anyhow!("MUVERA not configured — use new_with_muvera()"))?;

        let q_fde = muvera_encode(query_token_vecs, cfg.fde_dim, cfg.seed);

        // Exhaustive FDE scan
        let mut stmt = self.conn.prepare("SELECT doc_id, fde FROM colbert_muvera")?;
        let mut fde_scores: Vec<(i64, f32)> = stmt.query_map([], |row| {
            let doc_id: i64 = row.get(0)?;
            let bytes: Vec<u8> = row.get(1)?;
            Ok((doc_id, bytes))
        })?
        .filter_map(|r| r.ok())
        .map(|(doc_id, bytes)| {
            let fde: Vec<f32> = bytes.chunks_exact(4)
                .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                .collect();
            let sim = cosine_sim(&q_fde, &fde);
            (doc_id, sim)
        })
        .collect();

        fde_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        fde_scores.truncate(fde_top_n);

        let candidates: Vec<i64> = fde_scores.iter().map(|(id, _)| *id).collect();

        // ColBERT max-sim rerank on top-N
        let mut reranked: Vec<(i64, f32)> = candidates.iter().filter_map(|&doc_id| {
            // Try i8 first, fall back to f32
            let score = if let Ok(doc_q) = self.load_vecs_i8(doc_id) {
                let query_q: Vec<(Vec<i8>, f32)> = query_token_vecs.iter()
                    .map(|v| crate::quant::quant_i8(v))
                    .collect();
                max_sim_i8(&query_q, &doc_q)
            } else if let Ok(doc_vecs) = self.load_vecs(doc_id) {
                max_sim(query_token_vecs, &doc_vecs)
            } else {
                return None;
            };
            Some((doc_id, score))
        }).collect();

        reranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        reranked.truncate(k);
        Ok(reranked)
    }

    /// Store pre-computed token vecs as int8-quantized.
    /// vecs_i8 BLOB: raw i8 bytes (n_tokens × dim, row-major).
    /// scales BLOB: n_tokens × f32-le.
    pub fn add_colbert_i8(&self, doc_id: i64, vecs: Vec<Vec<f32>>) -> Result<()> {
        if vecs.is_empty() {
            return Ok(());
        }
        let dim = vecs[0].len();
        let n = vecs.len();
        let mut vecs_i8_bytes: Vec<u8> = Vec::with_capacity(n * dim);
        let mut scales_bytes: Vec<u8> = Vec::with_capacity(n * 4);
        for v in &vecs {
            let (q, scale) = quant_i8(v);
            // SAFETY: i8 → u8 transmute for blob storage
            vecs_i8_bytes.extend(q.iter().map(|&x| x as u8));
            scales_bytes.extend_from_slice(&scale.to_le_bytes());
        }
        self.conn.execute(
            "INSERT OR REPLACE INTO colbert_vecs_i8 (doc_id, vecs_i8, scales) VALUES (?1, ?2, ?3)",
            params![doc_id, vecs_i8_bytes, scales_bytes],
        )?;
        #[cfg(feature = "muvera")]
        self.store_fde(doc_id, &vecs)?;
        Ok(())
    }

    /// Rerank using i8 quantized vectors.
    pub fn colbert_rerank_i8(&self, query_text: &str, candidates: &[i64]) -> Result<Vec<(i64, f32)>> {
        let query_vecs = self.emb.embed_query(query_text)?;
        let query_q: Vec<(Vec<i8>, f32)> = query_vecs.iter().map(|v| quant_i8(v)).collect();
        let mut scores: Vec<(i64, f32)> = candidates.iter().filter_map(|&doc_id| {
            let doc_q = self.load_vecs_i8(doc_id).ok()?;
            let score = max_sim_i8(&query_q, &doc_q);
            Some((doc_id, score))
        }).collect();
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(scores)
    }

    fn load_vecs_i8(&self, doc_id: i64) -> Result<Vec<(Vec<i8>, f32)>> {
        let (vecs_i8_bytes, scales_bytes): (Vec<u8>, Vec<u8>) = self.conn.query_row(
            "SELECT vecs_i8, scales FROM colbert_vecs_i8 WHERE doc_id = ?1",
            params![doc_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let n = scales_bytes.len() / 4;
        if n == 0 || vecs_i8_bytes.len() % n != 0 {
            anyhow::bail!("corrupt i8 blob for doc_id={doc_id}");
        }
        let dim = vecs_i8_bytes.len() / n;
        let result = (0..n).map(|i| {
            let scale = f32::from_le_bytes(scales_bytes[i*4..i*4+4].try_into().unwrap());
            let q: Vec<i8> = vecs_i8_bytes[i*dim..(i+1)*dim].iter().map(|&x| x as i8).collect();
            (q, scale)
        }).collect();
        Ok(result)
    }

    /// Store pre-computed token vecs for a doc.
    pub fn add_colbert(&self, doc_id: i64, vecs: Vec<Vec<f32>>) -> Result<()> {
        #[cfg(feature = "muvera")]
        self.store_fde(doc_id, &vecs)?;
        let json = serde_json::to_vec(&vecs)?;
        let compressed = zstd::encode_all(json.as_slice(), 3)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO colbert_vecs (doc_id, vecs) VALUES (?1, ?2)",
            params![doc_id, compressed],
        )?;
        Ok(())
    }

    /// Embed text and store (f32 path).
    pub fn embed_and_add(&self, doc_id: i64, text: &str) -> Result<()> {
        let vecs = self.emb.embed_doc(text)?;
        self.add_colbert(doc_id, vecs)
    }

    /// Embed text and store as int8-quantised (preferred for production rerank).
    pub fn embed_and_add_i8(&self, doc_id: i64, text: &str) -> Result<()> {
        let vecs = self.emb.embed_doc(text)?;
        self.add_colbert_i8(doc_id, vecs)
    }

    /// Rerank ANN candidates by ColBERT max-sim. Returns sorted (doc_id, score) desc.
    pub fn colbert_rerank(&self, query_text: &str, candidates: &[i64]) -> Result<Vec<(i64, f32)>> {
        let query_vecs = self.emb.embed_query(query_text)?;
        let mut scores: Vec<(i64, f32)> = candidates.iter().filter_map(|&doc_id| {
            let doc_vecs = self.load_vecs(doc_id).ok()?;
            let score = max_sim(&query_vecs, &doc_vecs);
            Some((doc_id, score))
        }).collect();
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(scores)
    }

    fn load_vecs(&self, doc_id: i64) -> Result<Vec<Vec<f32>>> {
        let compressed: Vec<u8> = self.conn.query_row(
            "SELECT vecs FROM colbert_vecs WHERE doc_id = ?1",
            params![doc_id],
            |row| row.get(0),
        )?;
        let json = zstd::decode_all(compressed.as_slice())?;
        Ok(serde_json::from_slice(&json)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn smoke_10_docs() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        let store = ColbertStore::new(&conn)?;

        let docs = [
            "the quick brown fox jumps over the lazy dog",
            "neural information retrieval with dense representations",
            "ColBERT late interaction multi vector search",
            "apple banana cherry date elderberry fig",
            "tokio async rust runtime performance benchmark",
            "matrix multiplication linear algebra operations",
            "deep learning transformer attention mechanism",
            "database indexing b-tree hash join query",
            "the cat sat on the mat",
            "late interaction reranking colbert max sim score",
        ];

        for (i, doc) in docs.iter().enumerate() {
            store.embed_and_add(i as i64, doc)?;
        }

        let query = "ColBERT late interaction reranking";
        let candidates: Vec<i64> = (0..10).collect();
        let ranked = store.colbert_rerank(query, &candidates)?;

        assert_eq!(ranked.len(), 10);
        // doc 2 ("ColBERT late interaction...") and doc 9 ("late interaction reranking colbert...")
        // should rank in top-3
        let top3_ids: Vec<i64> = ranked[..3].iter().map(|(id, _)| *id).collect();
        assert!(
            top3_ids.contains(&2) || top3_ids.contains(&9),
            "Expected colbert-related docs in top-3, got: {:?}", ranked
        );
        // scores descending
        for w in ranked.windows(2) {
            assert!(w[0].1 >= w[1].1, "scores not sorted: {:?}", ranked);
        }

        tracing::info!("ColBERT smoke top-3: {:?}", &ranked[..3]);
        Ok(())
    }

    #[test]
    fn i8_store_and_rerank() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        let store = ColbertStore::new(&conn)?;

        let docs = [
            "ColBERT late interaction multi vector search",
            "late interaction reranking colbert max sim score",
            "neural information retrieval with dense representations",
            "apple banana cherry date elderberry fig",
        ];
        for (i, doc) in docs.iter().enumerate() {
            let vecs = store.emb.embed_doc(doc)?;
            store.add_colbert_i8(i as i64, vecs)?;
        }

        let query = "ColBERT late interaction reranking";
        let candidates: Vec<i64> = (0..4).collect();
        let ranked = store.colbert_rerank_i8(query, &candidates)?;
        assert_eq!(ranked.len(), 4);
        let top2: Vec<i64> = ranked[..2].iter().map(|(id, _)| *id).collect();
        assert!(top2.contains(&0) || top2.contains(&1),
            "Expected colbert docs in top-2, got: {:?}", ranked);
        for w in ranked.windows(2) {
            assert!(w[0].1 >= w[1].1);
        }
        Ok(())
    }

    #[test]
    fn bench_1000_pairs_i8_vs_f32() -> Result<()> {
        use std::time::Instant;
        let conn = Connection::open_in_memory()?;
        let store = ColbertStore::new(&conn)?;

        // pre-compute 10 docs, embed them
        let docs: Vec<&str> = vec![
            "the quick brown fox", "neural retrieval dense", "ColBERT late interaction",
            "apple banana cherry", "tokio async rust", "matrix multiplication",
            "deep learning transformer", "database indexing btree", "the cat sat mat",
            "late interaction colbert score",
        ];
        for (i, doc) in docs.iter().enumerate() {
            let vecs = store.emb.embed_doc(doc)?;
            store.add_colbert(i as i64, vecs.clone())?;
            store.add_colbert_i8(i as i64, vecs)?;
        }
        let query = "ColBERT reranking late interaction";
        let candidates: Vec<i64> = (0..10).collect();
        const ITERS: usize = 100;

        let t0 = Instant::now();
        for _ in 0..ITERS { store.colbert_rerank(query, &candidates)?; }
        let f32_ms = t0.elapsed().as_secs_f64() * 1000.0 / ITERS as f64;

        let t1 = Instant::now();
        for _ in 0..ITERS { store.colbert_rerank_i8(query, &candidates)?; }
        let i8_ms = t1.elapsed().as_secs_f64() * 1000.0 / ITERS as f64;

        // accuracy: compare top-3 ordering
        let f32_ranked = store.colbert_rerank(query, &candidates)?;
        let i8_ranked = store.colbert_rerank_i8(query, &candidates)?;
        let f32_top3: Vec<i64> = f32_ranked[..3].iter().map(|(id,_)| *id).collect();
        let i8_top3: Vec<i64> = i8_ranked[..3].iter().map(|(id,_)| *id).collect();
        let overlap = f32_top3.iter().filter(|id| i8_top3.contains(id)).count();
        // at least 2/3 top-3 must match (NDCG-proxy)
        assert!(overlap >= 2, "top-3 overlap too low: f32={f32_top3:?} i8={i8_top3:?}");

        eprintln!("BENCH f32={f32_ms:.3}ms i8={i8_ms:.3}ms ratio={:.2}× top3-overlap={overlap}/3",
            f32_ms / i8_ms);
        Ok(())
    }

    #[cfg(feature = "muvera")]
    #[test]
    fn muvera_two_tier_r10_smoke() -> Result<()> {
        use std::time::Instant;
        use super::MuveraConfig;
        use crate::muvera::muvera_encode;
        use crate::kernel::max_sim;

        // Build structured synthetic docs: query has dim-0 dominant.
        // Top-K docs have high dim-0; irrelevant docs have high other dims.
        // This ensures FDE and ColBERT max-sim agree (both are dot-product-based).
        let dim = 128usize;
        let n_docs = 100i64;
        let n_tokens = 4usize;
        let fde_seed = 42u64;
        let fde_dim = 512usize;

        // query token vecs: unit vector in dim-0
        let mut q_tok = vec![vec![0.0f32; dim]; n_tokens];
        for t in &mut q_tok {
            t[0] = 1.0; // all tokens point at dim-0
        }

        // Doc token vecs: relevant docs point at dim-0 with score proportional to match
        // irrelevant docs point at dim-1..99 (orthogonal to query)
        let make_doc_vecs = |doc_id: i64| -> Vec<Vec<f32>> {
            (0..n_tokens).map(|_| {
                let mut v = vec![0.0f32; dim];
                if doc_id < 10 {
                    // relevant: strong dim-0 signal
                    v[0] = 0.9;
                    v[(doc_id as usize + 1) % dim] = 0.1;
                } else {
                    // irrelevant: orthogonal dims
                    let d = ((doc_id as usize) % (dim - 1)) + 1;
                    v[d] = 1.0;
                }
                let norm = v.iter().map(|x| x*x).sum::<f32>().sqrt().max(1e-9);
                v.iter_mut().for_each(|x| *x /= norm);
                v
            }).collect()
        };

        // Build store with synthetic vecs directly (bypass embedder)
        let conn = Connection::open_in_memory()?;
        let cfg = MuveraConfig { fde_dim, seed: fde_seed };
        let store = ColbertStore::new_with_muvera(&conn, cfg)?;

        for doc_id in 0..n_docs {
            let vecs = make_doc_vecs(doc_id);
            store.add_colbert(doc_id, vecs)?;
        }

        let all_candidates: Vec<i64> = (0..n_docs).collect();

        // Full ColBERT rerank (ground truth)
        let t_full = Instant::now();
        let full_ranked: Vec<(i64, f32)> = all_candidates.iter().filter_map(|&doc_id| {
            let doc_vecs = store.load_vecs(doc_id).ok()?;
            let score = max_sim(&q_tok, &doc_vecs);
            Some((doc_id, score))
        }).collect::<Vec<_>>().into_iter().collect();
        let mut full_sorted = full_ranked;
        full_sorted.sort_by(|a,b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let full_ms = t_full.elapsed().as_secs_f64() * 1000.0;
        let full_top10: Vec<i64> = full_sorted[..10].iter().map(|(id,_)| *id).collect();

        // MUVERA two-tier: FDE top-30 → ColBERT rerank → top-10
        let fde_top_n = 30usize;
        let t_muv = Instant::now();
        let muv_ranked = store.search_muvera_then_rerank(&q_tok, 10, fde_top_n)?;
        let muv_ms = t_muv.elapsed().as_secs_f64() * 1000.0;
        let muv_top10: Vec<i64> = muv_ranked.iter().map(|(id,_)| *id).collect();

        // R@10: how many of full top-10 recovered by MUVERA
        let overlap = muv_top10.iter().filter(|id| full_top10.contains(id)).count();
        let recall_at_10 = overlap as f32 / 10.0;

        eprintln!(
            "MUVERA smoke: full={full_ms:.1}ms muvera={muv_ms:.1}ms speedup={:.1}× R@10={recall_at_10:.2} (overlap={overlap}/10)",
            full_ms / muv_ms.max(0.001)
        );
        eprintln!("  full top10:   {full_top10:?}");
        eprintln!("  muvera top10: {muv_top10:?}");

        assert!(
            recall_at_10 >= 0.95,
            "R@10 too low: {recall_at_10:.2} (overlap={overlap}/10). full={full_top10:?} muv={muv_top10:?}"
        );

        Ok(())
    }
}
