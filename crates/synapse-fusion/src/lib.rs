/// DocId mirrors synapse-core's i64 row-id (no hard dep on synapse-core).
pub type DocId = i64;

/// Reciprocal-Rank-Fusion over two ranked lists (dense + ColBERT).
///
/// Each input slice is `(doc_id, score)` **already sorted descending** by score.
/// `k_constant` is the RRF smoothing constant (paper default: 60.0).
///
/// Returns a new vec sorted descending by fused RRF score.
pub fn muvera_rrf(
    dense: &[(DocId, f32)],
    colbert: &[(DocId, f32)],
    k_constant: f32,
) -> Vec<(DocId, f32)> {
    use std::collections::HashMap;
    let mut scores: HashMap<DocId, f32> = HashMap::new();

    for (rank, (doc_id, _)) in dense.iter().enumerate() {
        *scores.entry(*doc_id).or_insert(0.0) += 1.0 / (k_constant + rank as f32 + 1.0);
    }
    for (rank, (doc_id, _)) in colbert.iter().enumerate() {
        *scores.entry(*doc_id).or_insert(0.0) += 1.0 / (k_constant + rank as f32 + 1.0);
    }

    let mut result: Vec<(DocId, f32)> = scores.into_iter().collect();
    result.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    result
}

/// Full pipeline stub — returns fused ranking from top-k dense and ColBERT candidates.
///
/// When the `fusion` feature is disabled this still compiles; callers provide
/// pre-computed ranked lists. The `query`, `dense_top`, and `colbert_rerank_top`
/// params are placeholders for the wired-up version.
///
/// # Arguments
/// * `dense_results`   – ranked vec from the dense (vec) retrieval leg
/// * `colbert_results` – ranked vec from the ColBERT late-interaction leg
/// * `dense_top`       – how many dense results to consider
/// * `colbert_rerank_top` – how many ColBERT results to consider
pub fn full_pipeline(
    dense_results: &[(DocId, f32)],
    colbert_results: &[(DocId, f32)],
    dense_top: usize,
    colbert_rerank_top: usize,
) -> Vec<DocId> {
    let dense = &dense_results[..dense_top.min(dense_results.len())];
    let colbert = &colbert_results[..colbert_rerank_top.min(colbert_results.len())];
    muvera_rrf(dense, colbert, 60.0)
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

// ── Full E2E pipeline (feature = "fusion-full") ───────────────────────────────

/// In-memory dense document store for the E2E pipeline.
///
/// Each document is represented by a pre-computed L2-normalised f32 vector.
/// ANN search is exhaustive (brute-force cosine) — adequate for top-100 from
/// moderate corpora; replace with usearch/hnsw once wired.
pub struct DenseStore {
    docs: Vec<(DocId, Vec<f32>)>,
}

impl DenseStore {
    pub fn new() -> Self {
        Self { docs: Vec::new() }
    }

    /// Add a document with its embedding (must be L2-normalised).
    pub fn add(&mut self, id: DocId, vec: Vec<f32>) {
        self.docs.push((id, vec));
    }

    /// Brute-force cosine top-k.
    pub fn search(&self, query: &[f32], k: usize) -> Vec<(DocId, f32)> {
        let mut scored: Vec<(DocId, f32)> = self
            .docs
            .iter()
            .map(|(id, v)| {
                let dot: f32 = v.iter().zip(query).map(|(a, b)| a * b).sum();
                (*id, dot)
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        scored
    }
}

impl Default for DenseStore {
    fn default() -> Self { Self::new() }
}

/// Full MUVERA pipeline result.
pub struct MuveraResult {
    /// Ranked doc ids (top-k after ColBERT rerank), index 0 = best.
    pub ranked: Vec<DocId>,
    /// Per-stage latencies in microseconds.
    pub latency_us: MuveraLatency,
}

pub struct MuveraLatency {
    pub dense_ann_us:    u64,
    pub splade_bmp_us:   u64,
    pub rrf_fuse_us:     u64,
    pub colbert_rerank_us: u64,
}

/// End-to-end MUVERA search.
///
/// # Arguments
/// * `query`        — raw text query
/// * `query_vec`    — pre-computed L2-normalised dense embedding of `query`
/// * `dense`        — populated [`DenseStore`]
/// * `bmp`          — populated [`synapse_splade::BlockMaxIndex`] (flush() already called)
/// * `splade_enc`   — [`synapse_splade::SpladeEncoder`] for query encoding
/// * `colbert_conn` — open rusqlite connection with colbert_vecs_i8 table populated
/// * `k`            — final top-k to return
#[cfg(feature = "fusion-full")]
pub fn search_muvera_full(
    query: &str,
    query_vec: &[f32],
    dense: &DenseStore,
    bmp: &synapse_splade::BlockMaxIndex,
    splade_enc: &synapse_splade::SpladeEncoder,
    colbert_conn: &rusqlite::Connection,
    k: usize,
) -> anyhow::Result<MuveraResult> {
    use std::time::Instant;

    // ── Stage 1: Dense ANN top-100 ────────────────────────────────────────────
    let t0 = Instant::now();
    let dense_top = dense.search(query_vec, 100);
    let dense_ann_us = t0.elapsed().as_micros() as u64;

    // ── Stage 2: SPLADE BMP top-100 ───────────────────────────────────────────
    let t1 = Instant::now();
    let q_sparse = splade_enc.encode(query)?;
    let splade_raw = bmp.search_topk(&q_sparse, 100);
    // Convert BlockMaxIndex DocId (u64) to i64
    let sparse_top: Vec<(DocId, f32)> = splade_raw
        .iter()
        .map(|&(id, s)| (id as DocId, s))
        .collect();
    let splade_bmp_us = t1.elapsed().as_micros() as u64;

    // ── Stage 3: RRF fuse → top-50 ────────────────────────────────────────────
    let t2 = Instant::now();
    let fused = muvera_rrf(&dense_top, &sparse_top, 60.0);
    let top50: Vec<DocId> = fused.iter().take(50).map(|(id, _)| *id).collect();
    let rrf_fuse_us = t2.elapsed().as_micros() as u64;

    // ── Stage 4: ColBERT i8 rerank top-50 → top-k ────────────────────────────
    let t3 = Instant::now();
    let store = synapse_colbert::ColbertStore::new(colbert_conn)?;
    let reranked = store.colbert_rerank_i8(query, &top50)?;
    let ranked: Vec<DocId> = reranked.iter().take(k).map(|(id, _)| *id).collect();
    let colbert_rerank_us = t3.elapsed().as_micros() as u64;

    Ok(MuveraResult {
        ranked,
        latency_us: MuveraLatency {
            dense_ann_us,
            splade_bmp_us,
            rrf_fuse_us,
            colbert_rerank_us,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrf_merges_disjoint_lists() {
        // doc A is rank-1 in dense, doc B is rank-1 in colbert, doc C appears in both
        let dense = vec![(1, 0.9), (3, 0.7), (2, 0.5)];
        let colbert = vec![(2, 0.95), (3, 0.8), (4, 0.6)];
        let fused = muvera_rrf(&dense, &colbert, 60.0);

        // doc 3 appears at rank 2 in both lists → highest RRF score
        let ids: Vec<DocId> = fused.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&3));
        let doc3_score = fused.iter().find(|(id, _)| *id == 3).unwrap().1;
        // rank-2 in both: 1/(60+2) + 1/(60+2) = 2/62 ≈ 0.03226
        let expected = 2.0 / 62.0;
        assert!((doc3_score - expected).abs() < 1e-5, "doc3 score {doc3_score} != {expected}");
    }

    #[test]
    fn rrf_single_list_dominates_when_other_empty() {
        let dense = vec![(10, 1.0), (20, 0.5)];
        let colbert: Vec<(DocId, f32)> = vec![];
        let fused = muvera_rrf(&dense, &colbert, 60.0);

        assert_eq!(fused.len(), 2);
        // rank-1 doc should have higher RRF than rank-2
        assert!(fused[0].1 > fused[1].1);
        assert_eq!(fused[0].0, 10);
    }

    #[test]
    fn dense_store_search() {
        let mut store = DenseStore::new();
        // 3-dim vecs, all L2-norm ~1
        store.add(1, vec![1.0, 0.0, 0.0]);
        store.add(2, vec![0.0, 1.0, 0.0]);
        store.add(3, vec![0.707, 0.707, 0.0]);
        let results = store.search(&[1.0, 0.0, 0.0], 2);
        assert_eq!(results[0].0, 1);
        assert!(results[0].1 > results[1].1);
    }

    #[cfg(feature = "fusion-full")]
    #[test]
    fn muvera_e2e_smoke() -> anyhow::Result<()> {
        use std::time::Instant;
        use synapse_splade::{BlockMaxIndex, SpladeEncoder};
        use synapse_colbert::ColbertStore;
        use rusqlite::Connection;

        let docs = [
            (0i64, "ColBERT late interaction retrieval multi vector dense"),
            (1i64, "neural sparse SPLADE inverted index efficient search"),
            (2i64, "apple banana cherry fruit salad recipe"),
            (3i64, "tokio async runtime rust performance concurrency"),
            (4i64, "late interaction reranking colbert max sim score neural"),
            (5i64, "transformer attention mechanism bert embeddings"),
            (6i64, "database btree index scan query optimizer"),
            (7i64, "dense retrieval embedding similarity cosine dot product"),
            (8i64, "information retrieval SPLADE sparse expansion"),
            (9i64, "deep learning text classification sentiment analysis"),
        ];

        // Build dense store (random 8-dim stub, make doc 0 and 4 colbert-like)
        let mut dense = DenseStore::new();
        let query_vec: Vec<f32> = vec![0.9, 0.4, 0.1, 0.0, 0.0, 0.0, 0.0, 0.0];
        let norm = query_vec.iter().map(|x| x*x).sum::<f32>().sqrt();
        let query_vec: Vec<f32> = query_vec.iter().map(|x| x/norm).collect();

        for &(id, text) in &docs {
            // Simple bag-of-words 8-dim fingerprint
            let mut v = vec![0.0f32; 8];
            for (i, w) in text.split_whitespace().enumerate() {
                v[i % 8] += (w.len() as f32) / 10.0;
            }
            let n = v.iter().map(|x| x*x).sum::<f32>().sqrt().max(1e-6);
            let v: Vec<f32> = v.iter().map(|x| x/n).collect();
            dense.add(id, v);
        }

        // Build SPLADE BMP
        let enc = SpladeEncoder::default();
        let mut bmp = BlockMaxIndex::new(4);
        for &(id, text) in &docs {
            let sv = enc.encode(text)?;
            bmp.add_doc(id as u64, &sv);
        }
        bmp.flush();

        // Build ColBERT store (i8 path — required by colbert_rerank_i8)
        let conn = Connection::open_in_memory()?;
        {
            let store = ColbertStore::new(&conn)?;
            for &(id, text) in &docs {
                store.embed_and_add_i8(id, text)?;
            }
        }

        let t0 = Instant::now();
        let result = search_muvera_full(
            "ColBERT late interaction reranking",
            &query_vec,
            &dense,
            &bmp,
            &enc,
            &conn,
            5,
        )?;
        let total_us = t0.elapsed().as_micros();

        assert!(!result.ranked.is_empty(), "ranked must be non-empty");
        assert!(result.ranked.len() <= 5);
        // doc 0 or 4 should appear in top-3 (both contain "late interaction colbert")
        let top3 = &result.ranked[..result.ranked.len().min(3)];
        assert!(
            top3.contains(&0) || top3.contains(&4),
            "Expected colbert docs in top-3, got: {:?}", result.ranked
        );

        eprintln!(
            "MUVERA E2E latency: dense={}µs splade={}µs rrf={}µs colbert={}µs total={}µs",
            result.latency_us.dense_ann_us,
            result.latency_us.splade_bmp_us,
            result.latency_us.rrf_fuse_us,
            result.latency_us.colbert_rerank_us,
            total_us,
        );
        Ok(())
    }
}
