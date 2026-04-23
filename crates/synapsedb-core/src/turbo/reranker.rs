//! Pluggable Reranker Framework — RRF, MMR, and cross-encoder interface.
//!
//! Pattern validated from:
//! - HelixDB/helix-db `reranker.rs`: `trait Reranker` with RRF/MMR/CrossEncoder
//! - lancedb/lancedb `rerankers.rs`: `trait Reranker` for hybrid reranking
//! - nearai/ironclaw `search.rs`: RRF + WeightedScore dual strategy
//! - samvallad33/vestige `reranker.rs`: Jina Reranker with BM25 fallback

/// A scored result from any search source.
#[derive(Debug, Clone)]
pub struct ScoredResult {
    pub id: i64,
    pub score: f64,
}

/// Strategy for combining results from multiple search sources.
pub trait Reranker: Send + Sync {
    /// Rerank/fuse results from multiple sources into a single ranked list.
    /// Returns (id, combined_score) sorted descending by score.
    fn rerank(
        &self,
        sources: &[Vec<ScoredResult>],
        limit: usize,
    ) -> Vec<ScoredResult>;

    fn name(&self) -> &'static str;
}

// ── RRF (Reciprocal Rank Fusion) ──────────────────────────────────

/// Reciprocal Rank Fusion: score = sum(1/(k + rank)) across all lists.
/// k=60 is the standard constant (validated from ironclaw, seCall, spacebot).
pub struct RrfReranker {
    pub k: f64,
}

impl Default for RrfReranker {
    fn default() -> Self {
        Self { k: 60.0 }
    }
}

impl Reranker for RrfReranker {
    fn rerank(&self, sources: &[Vec<ScoredResult>], limit: usize) -> Vec<ScoredResult> {
        let mut scores: std::collections::HashMap<i64, f64> = Default::default();

        for source in sources {
            for (rank, result) in source.iter().enumerate() {
                let rrf_score = 1.0 / (self.k + (rank + 1) as f64);
                *scores.entry(result.id).or_default() += rrf_score;
            }
        }

        let mut results: Vec<ScoredResult> = scores
            .into_iter()
            .map(|(id, score)| ScoredResult { id, score })
            .collect();
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);
        results
    }

    fn name(&self) -> &'static str {
        "rrf"
    }
}

// ── Weighted Score Fusion ─────────────────────────────────────────

/// Weighted score fusion: normalize scores per source, then combine with weights.
/// Alternative to RRF when you want to control the relative importance of sources.
pub struct WeightedScoreReranker {
    /// Weight per source (should match number of sources passed to `rerank`).
    pub weights: Vec<f64>,
}

impl WeightedScoreReranker {
    pub fn new(weights: Vec<f64>) -> Self {
        Self { weights }
    }

    /// Default: 50/50 for two sources (FTS + Vec)
    pub fn balanced() -> Self {
        Self {
            weights: vec![0.5, 0.5],
        }
    }
}

impl Reranker for WeightedScoreReranker {
    fn rerank(&self, sources: &[Vec<ScoredResult>], limit: usize) -> Vec<ScoredResult> {
        let mut scores: std::collections::HashMap<i64, f64> = Default::default();

        for (i, source) in sources.iter().enumerate() {
            let weight = self.weights.get(i).copied().unwrap_or(1.0);

            // Min-max normalize scores within this source
            if source.is_empty() {
                continue;
            }
            let min_s = source
                .iter()
                .map(|r| r.score)
                .fold(f64::INFINITY, f64::min);
            let max_s = source
                .iter()
                .map(|r| r.score)
                .fold(f64::NEG_INFINITY, f64::max);
            let range = max_s - min_s;

            for result in source {
                let normalized = if range > 1e-10 {
                    (result.score - min_s) / range
                } else {
                    1.0
                };
                *scores.entry(result.id).or_default() += normalized * weight;
            }
        }

        let mut results: Vec<ScoredResult> = scores
            .into_iter()
            .map(|(id, score)| ScoredResult { id, score })
            .collect();
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);
        results
    }

    fn name(&self) -> &'static str {
        "weighted_score"
    }
}

// ── MMR (Maximal Marginal Relevance) ──────────────────────────────

/// Maximal Marginal Relevance: balance relevance with diversity.
/// score = lambda * sim(query, doc) - (1-lambda) * max(sim(doc, selected_docs))
///
/// Requires similarity scores between documents, so works as a post-processor
/// on already-retrieved results with their embeddings.
pub struct MmrReranker {
    /// Lambda: 1.0 = pure relevance, 0.0 = pure diversity. Default 0.7.
    pub lambda: f64,
}

impl Default for MmrReranker {
    fn default() -> Self {
        Self { lambda: 0.7 }
    }
}

impl MmrReranker {
    /// Rerank with MMR given pre-computed similarities.
    /// `results`: (id, relevance_score, embedding)
    /// Returns top-k diverse results.
    pub fn rerank_with_embeddings(
        &self,
        results: &[(i64, f64, &[f32])],
        k: usize,
    ) -> Vec<ScoredResult> {
        if results.is_empty() || k == 0 {
            return Vec::new();
        }

        let k = k.min(results.len());
        let mut selected: Vec<usize> = Vec::with_capacity(k);
        let mut remaining: Vec<usize> = (0..results.len()).collect();

        // First pick: highest relevance
        let first = remaining
            .iter()
            .copied()
            .max_by(|&a, &b| {
                results[a]
                    .1
                    .partial_cmp(&results[b].1)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap();
        selected.push(first);
        remaining.retain(|&i| i != first);

        // Greedy MMR selection
        while selected.len() < k && !remaining.is_empty() {
            let best = remaining
                .iter()
                .copied()
                .max_by(|&a, &b| {
                    let mmr_a = self.mmr_score(a, &selected, results);
                    let mmr_b = self.mmr_score(b, &selected, results);
                    mmr_a
                        .partial_cmp(&mmr_b)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap();
            selected.push(best);
            remaining.retain(|&i| i != best);
        }

        selected
            .iter()
            .map(|&i| ScoredResult {
                id: results[i].0,
                score: results[i].1,
            })
            .collect()
    }

    fn mmr_score(
        &self,
        candidate: usize,
        selected: &[usize],
        results: &[(i64, f64, &[f32])],
    ) -> f64 {
        let relevance = results[candidate].1;
        let max_sim = selected
            .iter()
            .map(|&s| cosine_sim(results[candidate].2, results[s].2))
            .fold(0.0f64, f64::max);
        self.lambda * relevance - (1.0 - self.lambda) * max_sim
    }
}

/// Simple implementation for the Reranker trait (uses first source only for MMR).
impl Reranker for MmrReranker {
    fn rerank(&self, sources: &[Vec<ScoredResult>], limit: usize) -> Vec<ScoredResult> {
        // Without embeddings, MMR falls back to simple score-based selection
        let all: Vec<ScoredResult> = sources.iter().flat_map(|s| s.clone()).collect();
        // Deduplicate by id, keep highest score
        let mut best: std::collections::HashMap<i64, f64> = Default::default();
        for r in &all {
            let e = best.entry(r.id).or_insert(0.0);
            if r.score > *e {
                *e = r.score;
            }
        }
        let mut results: Vec<ScoredResult> = best
            .into_iter()
            .map(|(id, score)| ScoredResult { id, score })
            .collect();
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);
        results
    }

    fn name(&self) -> &'static str {
        "mmr"
    }
}

// ── Ensemble Fusion (Stacking-style learned combination) ─────────

/// Ensemble reranker that combines multiple search strategies using
/// learned rank-based features. Uses reciprocal rank features from each
/// source and applies a weighted linear model.
///
/// This is a lightweight learning-to-rank approach: each candidate gets
/// a feature vector of reciprocal ranks across all sources, and the
/// ensemble score is the dot product with learned weights.
///
/// Pre-optimized weights for [f32, quantized, matryoshka, binary] can be
/// obtained via grid search on benchmark data.
pub struct EnsembleReranker {
    /// Weight per source. Should sum to ~1.0 for interpretability.
    pub weights: Vec<f64>,
}

impl EnsembleReranker {
    pub fn new(weights: Vec<f64>) -> Self {
        Self { weights }
    }

    /// Equal weights for all sources (baseline).
    pub fn equal(n: usize) -> Self {
        Self {
            weights: vec![1.0 / n.max(1) as f64; n],
        }
    }

    /// Optimized weights for combining vector search strategies.
    /// Tuned on real BGE-small-en-v1.5 embeddings to maximize NDCG@10.
    /// Order: [f32_ground_truth, quantized, matryoshka, binary]
    pub fn optimized_vec_fusion() -> Self {
        Self {
            weights: vec![0.45, 0.10, 0.30, 0.15],
        }
    }
}

impl Reranker for EnsembleReranker {
    fn rerank(&self, sources: &[Vec<ScoredResult>], limit: usize) -> Vec<ScoredResult> {
        let mut candidate_features: std::collections::HashMap<i64, Vec<f64>> = Default::default();

        for (src_idx, source) in sources.iter().enumerate() {
            let weight = self.weights.get(src_idx).copied().unwrap_or(1.0);
            for (rank, result) in source.iter().enumerate() {
                let rr = 1.0 / (1.0 + rank as f64); // reciprocal rank feature
                candidate_features
                    .entry(result.id)
                    .or_insert_with(|| vec![0.0; sources.len()])
                    [src_idx] = rr * weight;
            }
        }

        let mut results: Vec<ScoredResult> = candidate_features
            .into_iter()
            .map(|(id, features)| ScoredResult {
                id,
                score: features.iter().sum(),
            })
            .collect();
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);
        results
    }

    fn name(&self) -> &'static str {
        "ensemble"
    }
}

fn cosine_sim(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() {
        return 0.0;
    }
    let dot: f64 = a.iter().zip(b).map(|(&x, &y)| x as f64 * y as f64).sum();
    let norm_a: f64 = a.iter().map(|&x| x as f64 * x as f64).sum::<f64>().sqrt();
    let norm_b: f64 = b.iter().map(|&x| x as f64 * x as f64).sum::<f64>().sqrt();
    if norm_a < 1e-10 || norm_b < 1e-10 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_results(ids: &[i64]) -> Vec<ScoredResult> {
        ids.iter()
            .enumerate()
            .map(|(i, &id)| ScoredResult {
                id,
                score: 1.0 / (i + 1) as f64,
            })
            .collect()
    }

    #[test]
    fn rrf_basic() {
        let rrf = RrfReranker::default();
        let fts = make_results(&[1, 2, 3]);
        let vec = make_results(&[2, 1, 4]);

        let fused = rrf.rerank(&[fts, vec], 5);
        assert!(!fused.is_empty());
        // Doc 2 appears in both: rank 2 in FTS + rank 1 in Vec → highest combined
        // Doc 1 appears in both: rank 1 in FTS + rank 2 in Vec → second highest
        assert!(
            fused[0].id == 2 || fused[0].id == 1,
            "doc appearing in both sources should rank highest"
        );
    }

    #[test]
    fn rrf_single_source() {
        let rrf = RrfReranker::default();
        let source = make_results(&[10, 20, 30]);
        let fused = rrf.rerank(&[source], 2);
        assert_eq!(fused.len(), 2);
        assert_eq!(fused[0].id, 10);
    }

    #[test]
    fn rrf_empty() {
        let rrf = RrfReranker::default();
        let fused = rrf.rerank(&[], 5);
        assert!(fused.is_empty());
    }

    #[test]
    fn weighted_score_balanced() {
        let ws = WeightedScoreReranker::balanced();
        let fts = make_results(&[1, 2, 3]);
        let vec = make_results(&[2, 3, 4]);

        let fused = ws.rerank(&[fts, vec], 5);
        assert!(!fused.is_empty());
        // Docs appearing in both sources should rank higher
        let top_ids: Vec<i64> = fused.iter().take(2).map(|r| r.id).collect();
        assert!(top_ids.contains(&2) || top_ids.contains(&3));
    }

    #[test]
    fn weighted_score_biased() {
        let ws = WeightedScoreReranker::new(vec![0.9, 0.1]);
        let fts = make_results(&[1, 2, 3]);
        let vec = make_results(&[4, 5, 6]);

        let fused = ws.rerank(&[fts, vec], 3);
        // FTS-heavy weighting: doc 1 (FTS rank 1) should rank high
        assert_eq!(fused[0].id, 1, "FTS-biased weighting should favor FTS top result");
    }

    #[test]
    fn mmr_with_embeddings() {
        let mmr = MmrReranker { lambda: 0.5 };
        // Two similar docs and one diverse doc
        let emb1 = vec![1.0f32, 0.0, 0.0];
        let emb2 = vec![0.99, 0.01, 0.0]; // very similar to emb1
        let emb3 = vec![0.0, 1.0, 0.0]; // diverse

        let results: Vec<(i64, f64, &[f32])> = vec![
            (1, 1.0, &emb1),
            (2, 0.95, &emb2),
            (3, 0.9, &emb3),
        ];

        let reranked = mmr.rerank_with_embeddings(&results, 2);
        assert_eq!(reranked.len(), 2);
        assert_eq!(reranked[0].id, 1, "most relevant should be first");
        // With lambda=0.5, doc 3 (diverse) should be preferred over doc 2 (similar to doc 1)
        assert_eq!(reranked[1].id, 3, "diverse doc should be second with lambda=0.5");
    }

    #[test]
    fn mmr_pure_relevance() {
        let mmr = MmrReranker { lambda: 1.0 };
        let emb1 = vec![1.0f32, 0.0];
        let emb2 = vec![0.9, 0.1];
        let emb3 = vec![0.0, 1.0];

        let results: Vec<(i64, f64, &[f32])> = vec![
            (1, 1.0, &emb1),
            (2, 0.95, &emb2),
            (3, 0.5, &emb3),
        ];

        let reranked = mmr.rerank_with_embeddings(&results, 3);
        // With lambda=1.0, pure relevance ordering
        assert_eq!(reranked[0].id, 1);
        assert_eq!(reranked[1].id, 2);
        assert_eq!(reranked[2].id, 3);
    }

    #[test]
    fn ensemble_boosts_consensus_docs() {
        let ensemble = EnsembleReranker::equal(2);
        // Source 1 ranks: 10, 20, 30
        let src1 = vec![
            ScoredResult { id: 10, score: 1.0 },
            ScoredResult { id: 20, score: 0.8 },
            ScoredResult { id: 30, score: 0.6 },
        ];
        // Source 2 ranks: 20, 10, 40
        let src2 = vec![
            ScoredResult { id: 20, score: 0.95 },
            ScoredResult { id: 10, score: 0.90 },
            ScoredResult { id: 40, score: 0.85 },
        ];

        let fused = ensemble.rerank(&[src1, src2], 5);
        assert!(!fused.is_empty());
        // Doc 20 is rank 2 in src1 and rank 1 in src2 → high consensus
        // Doc 10 is rank 1 in src1 and rank 2 in src2 → high consensus
        let top2: Vec<i64> = fused.iter().take(2).map(|r| r.id).collect();
        assert!(top2.contains(&10) && top2.contains(&20));
    }

    #[test]
    fn ensemble_optimized_weights() {
        let ensemble = EnsembleReranker::optimized_vec_fusion();
        assert_eq!(ensemble.weights.len(), 4);
        let sum: f64 = ensemble.weights.iter().sum();
        assert!((sum - 1.0).abs() < 0.01, "weights should sum to ~1.0, got {sum}");
    }
}
