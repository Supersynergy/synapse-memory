//! Hamming → HNSW cascade for large-corpus ANN search.
//!
//! Phase A: 1-bit Hamming sweep over all docs → top K_rough = K × rough_mult candidates.
//! Phase B: HNSW search restricted to those candidates via id-allowlist.
//! Phase C: f32 exact rerank of HNSW results (inherits from `AnnIndex::search_with_rerank`).
//!
//! Acceptance target: R@10 ≥ 0.95, p50 < 1 ms @ 1 M corpus / 128-dim.
//!
//! Note: UsearchIndex does not expose per-query id-allowlists natively.
//! Phase B is therefore implemented as a standard HNSW search with boosted ef
//! (ef = K_rough), which approximates "search over subset" on dense HNSW graphs.
//! True subset-search requires a custom usearch patch or IVF-PQ backend (PR-A2).

use crate::{AnnError, AnnIndex, SearchResults};

/// Configuration for the Hamming→HNSW cascade.
#[derive(Clone, Debug)]
pub struct CascadeConfig {
    /// Oversampling multiplier for Phase A (Hamming): K_rough = k * rough_mult.
    /// Larger = higher recall Phase A, slower Phase B.  Default 100.
    pub rough_mult: usize,
    /// ef_search boost for Phase B (HNSW).  Defaults to K_rough.
    /// Override to decouple HNSW expansion from rough_mult.
    pub hnsw_ef: Option<usize>,
}

impl Default for CascadeConfig {
    fn default() -> Self {
        Self {
            rough_mult: 100,
            hnsw_ef: None,
        }
    }
}

/// Sign-binarize a f32 slice into a bit-packed byte vec (MSB first).
pub fn binarize(v: &[f32]) -> Vec<u8> {
    let bpr = v.len().div_ceil(8);
    let mut out = vec![0u8; bpr];
    for (i, &x) in v.iter().enumerate() {
        if x > 0.0 {
            out[i / 8] |= 0x80u8 >> (i % 8);
        }
    }
    out
}

/// Hamming distance between two equal-length byte slices (bit-level).
#[inline]
pub fn hamming_u8(a: &[u8], b: &[u8]) -> u32 {
    a.iter().zip(b).map(|(&x, &y)| (x ^ y).count_ones()).sum()
}

/// Brute-force Hamming sweep: returns top-K_rough (id, hamming_dist) ascending.
pub fn hamming_topk(
    query_bits: &[u8],
    corpus_ids: &[u64],
    corpus_bits: &[u8], // row-major, bpr bytes per row
    bpr: usize,
    k_rough: usize,
) -> Vec<(u64, u32)> {
    debug_assert_eq!(corpus_bits.len(), corpus_ids.len() * bpr);
    // Collect all (dist, id) then partial-sort.
    let mut scores: Vec<(u32, u64)> = corpus_ids
        .iter()
        .enumerate()
        .map(|(i, &id)| {
            let row = &corpus_bits[i * bpr..(i + 1) * bpr];
            (hamming_u8(query_bits, row), id)
        })
        .collect();
    let k = k_rough.min(scores.len());
    if k == 0 {
        return vec![];
    }
    // Partial select to avoid full sort on large corpora.
    scores.select_nth_unstable(k - 1);
    scores.truncate(k);
    scores.sort_unstable();
    scores.iter().map(|&(d, id)| (id, d)).collect()
}

/// Run the 3-phase cascade on any `AnnIndex`.
///
/// `corpus_ids` / `corpus_bits` must be pre-built from the same vectors
/// inserted into `index`.  `bpr` = bytes-per-row = `dim.div_ceil(8)`.
///
/// Phase A: Hamming sweep → K_rough candidates.
/// Phase B: HNSW search with ef = max(K_rough, cfg.hnsw_ef) to approximate
///          subset search on the dense graph.
/// Phase C: truncate + sort (already handled by `search_with_rerank`).
pub fn cascade_search<I: AnnIndex>(
    index: &I,
    query: &[f32],
    k: usize,
    corpus_ids: &[u64],
    corpus_bits: &[u8],
    bpr: usize,
    cfg: &CascadeConfig,
) -> Result<SearchResults, AnnError> {
    let k_rough = k.saturating_mul(cfg.rough_mult).max(k * 2);
    // Phase A — Hamming candidate generation.
    let query_bits = binarize(query);
    let candidates = hamming_topk(&query_bits, corpus_ids, corpus_bits, bpr, k_rough);
    if candidates.is_empty() {
        return Ok(vec![]);
    }
    // Phase B — HNSW with boosted ef (approximates restricted subset search).
    let ef = cfg.hnsw_ef.unwrap_or(k_rough).max(k_rough);
    let mult = (ef / index.len().max(1)).clamp(2, 100);
    let mut hits = index.search_with_rerank(query, k, mult)?;

    // Phase C — keep only candidates that appeared in Phase A allowlist.
    // (optional: can skip for pure-HNSW throughput, rely on rerank quality)
    let allow: std::collections::HashSet<u64> = candidates.iter().map(|&(id, _)| id).collect();
    hits.retain(|(id, _)| allow.contains(id));
    // Pad from full HNSW if filtering left < k results.
    if hits.len() < k {
        let full = index.search(query, k)?;
        for item in full {
            if !hits.iter().any(|&(id, _)| id == item.0) {
                hits.push(item);
                if hits.len() >= k {
                    break;
                }
            }
        }
        hits.sort_unstable_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(k);
    }
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binarize_sign_correct() {
        let v = vec![1.0f32, -1.0, 0.5, -0.5];
        let b = binarize(&v);
        // bits: 1010 (MSB-first in byte 0)
        assert_eq!(b[0], 0b10100000);
    }

    #[test]
    fn hamming_identical_zero() {
        let a = vec![0xFFu8, 0x00, 0xAA];
        assert_eq!(hamming_u8(&a, &a), 0);
    }

    #[test]
    fn hamming_topk_returns_k() {
        let n = 200usize;
        let bpr = 8usize; // 64-bit signatures
        let ids: Vec<u64> = (0..n as u64).collect();
        let bits: Vec<u8> = (0..n)
            .flat_map(|i| {
                let v = i as u64;
                v.to_le_bytes().to_vec()
            })
            .collect();
        let query_bits = 42u64.to_le_bytes().to_vec();
        let top = hamming_topk(&query_bits, &ids, &bits, bpr, 10);
        assert_eq!(top.len(), 10);
        // id=42 must be top-1 (hamming dist 0)
        assert_eq!(top[0].0, 42);
        assert_eq!(top[0].1, 0);
    }
}
