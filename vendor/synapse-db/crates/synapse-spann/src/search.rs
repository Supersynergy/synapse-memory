//! SPANN query: top-nprobe centroids → scan posting lists → exact dot-product rerank.

use crate::{SearchHit, SearchResults, posting::MmapPostingList};

/// Find top-nprobe centroid indices by L2 distance to query.
pub fn nearest_centroids(centroids: &[Vec<f32>], query: &[f32], nprobe: usize) -> Vec<usize> {
    let mut dists: Vec<(usize, f32)> = centroids
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let d: f32 = c.iter().zip(query).map(|(a, b)| (a - b).powi(2)).sum();
            (i, d)
        })
        .collect();
    dists.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    dists.truncate(nprobe);
    dists.into_iter().map(|(i, _)| i).collect()
}

/// Exact dot-product (inner-product similarity; negate for max-IP nearest).
#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Scan posting lists for given cluster ids, return top-k by dot-product descending.
pub fn scan_and_rerank(
    posting_lists: &[MmapPostingList],
    cluster_ids: &[usize],
    query: &[f32],
    k: usize,
) -> SearchResults {
    let mut candidates: Vec<SearchHit> = Vec::new();
    for &cid in cluster_ids {
        if cid >= posting_lists.len() {
            continue;
        }
        for (docid, vec) in posting_lists[cid].entries() {
            let score = dot(&vec, query);
            candidates.push((docid, score));
        }
    }
    candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    candidates.dedup_by_key(|e| e.0);
    candidates.truncate(k);
    candidates
}
