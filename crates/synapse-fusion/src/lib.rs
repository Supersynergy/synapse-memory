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
}
