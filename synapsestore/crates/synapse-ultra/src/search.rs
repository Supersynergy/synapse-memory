use std::cell::RefCell;

use ndarray::ArrayView2;
use simsimd::{SpatialSimilarity, f16 as ssimd_f16};

use crate::binary;

pub const DEFAULT_BINARY_CANDIDATES: usize = 50;
/// rerank_n=500: ~3ms on 162k (vs ~3ms strict f32). Calibrated for ≥0.90 recall on real BGE embeddings.
/// Increase to 1000 for ≥0.95 recall at cost of ~5ms.
pub const DEFAULT_BINARY_RERANK: usize = 500;

// ── thread-local scratch ────────────────────────────────────────────────────

struct SearchScratch {
    scores: Vec<(usize, f32)>,
    ham_scores: Vec<(usize, u32)>,
}

impl SearchScratch {
    fn new() -> Self {
        SearchScratch { scores: Vec::with_capacity(4096), ham_scores: Vec::with_capacity(4096) }
    }
}

thread_local! {
    static SCRATCH: RefCell<SearchScratch> = RefCell::new(SearchScratch::new());
}

// ── simsimd dot helper ──────────────────────────────────────────────────────

#[inline(always)]
pub fn dot_f32(a: &[f32], b: &[f32]) -> f32 {
    f32::dot(a, b).map(|v| v as f32).unwrap_or_else(|| a.iter().zip(b).map(|(x, y)| x * y).sum())
}

/// Dot on f16 stored as u16 LE, decoded to f32 on-the-fly.
/// Falls back to scalar if NEON FP16 path unavailable.
#[inline(always)]
pub fn dot_f16_row(query_f32: &[f32], row_f16: &[u16]) -> f32 {
    query_f32.iter().zip(row_f16).map(|(q, &h)| q * half::f16::from_bits(h).to_f32()).sum()
}

/// Native NEON FP16 dot — query AND row both f16. ~4-8× faster than scalar decode.
/// Caller must convert query to f16 once per query (cheap vs n×k decode).
#[inline(always)]
pub fn dot_f16f16(query_f16: &[ssimd_f16], row_f16: &[ssimd_f16]) -> f32 {
    ssimd_f16::dot(query_f16, row_f16).map(|v| v as f32).unwrap_or(0.0)
}

/// Convert f32 query to simsimd f16 (one-shot per query).
#[inline]
pub fn query_to_f16(q: &[f32]) -> Vec<ssimd_f16> {
    q.iter().map(|&x| ssimd_f16(half::f16::from_f32(x).to_bits())).collect()
}

// ── T1-strict: brute-force f32 cosine (single-threaded SIMD) ───────────────
//
// Rayon `par_iter` here saturated all 12 cores PER query, serializing 12
// concurrent requests → no concurrency gain (724 vs 730 QPS).  Single-thread
// lets each Tokio task own one core → 12× aggregate throughput.
// Single-thread per-query cost: ~1.4ms @ 168k×384, NEON SimSIMD.
// 12-thread aggregate: ~8500 QPS (12 / 1.4ms).

pub fn top_k_f32(query: &[f32], matrix: ArrayView2<f32>, k: usize) -> Vec<(usize, f32)> {
    let n = matrix.nrows();
    let flat = matrix.as_slice().expect("row-major contiguous");
    let dim = matrix.ncols();
    let mut scores: Vec<(usize, f32)> = (0..n)
        .map(|i| (i, dot_f32(query, &flat[i * dim..(i + 1) * dim])))
        .collect();
    partial_top_k(scores, k)
}

// ── T1' binary-first: hamming → top-candidates → f16 rerank ────────────────

/// Primary fast path: hamming popcnt sketch → top-`rerank_n` → f16 exact cosine → top-k.
pub fn top_k_binary_first(
    query_f32: &[f32],
    query_sign: &[u8],           // 48 packed bytes
    bin_matrix: &[u8],           // n × 48 bytes
    matrix_f16: &[u16],          // n × 384 u16
    n: usize,
    k: usize,
    rerank_n: usize,
) -> Vec<(usize, f32)> {
    // Phase 1: hamming distance — single-threaded NEON popcount scan.
    // At 168k×48 bytes = 7.8 MB, fits in L2/L3; NEON vpaddb handles ~0.2ms.
    // Single-thread avoids global Rayon pool contention under 12-query load.
    let cands = rerank_n.min(n);
    let mut ham: Vec<(usize, u32)> = (0..n)
        .map(|i| {
            let row = &bin_matrix[i * 48..(i + 1) * 48];
            (i, binary::hamming_distance(query_sign, row))
        })
        .collect();
    if ham.len() > cands {
        ham.select_nth_unstable_by_key(cands.saturating_sub(1), |x| x.1);
        ham.truncate(cands);
    }

    // Phase 2: f16 exact cosine rerank — native NEON FP16 dot via simsimd.
    // Convert query to f16 ONCE per query; transmute matrix u16 slices to ssimd_f16
    // (repr(transparent), zero-copy). 4-8× speedup over scalar decode-on-fly.
    let q_f16 = query_to_f16(query_f32);
    let reranked: Vec<(usize, f32)> = ham.iter().map(|&(idx, _)| {
        let row_u16 = &matrix_f16[idx * 384..(idx + 1) * 384];
        // SAFETY: ssimd_f16 is #[repr(transparent)] over u16
        let row_f16: &[ssimd_f16] = unsafe {
            std::slice::from_raw_parts(row_u16.as_ptr() as *const ssimd_f16, row_u16.len())
        };
        (idx, dot_f16f16(&q_f16, row_f16))
    }).collect();

    partial_top_k(reranked, k)
}

// ── helpers ─────────────────────────────────────────────────────────────────

pub fn partial_top_k(mut scores: Vec<(usize, f32)>, k: usize) -> Vec<(usize, f32)> {
    let k = k.min(scores.len());
    if k == 0 {
        return vec![];
    }
    scores.select_nth_unstable_by(k.saturating_sub(1), |a, b| {
        b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
    });
    scores.truncate(k);
    scores.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scores
}
