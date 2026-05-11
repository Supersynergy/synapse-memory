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
    let scores: Vec<(usize, f32)> = (0..n)
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

// ── BLAS GEMM batch search (macOS Accelerate / Linux ndarray fallback) ──────
//
// Computes Q @ M^T in one SGEMM call — hits AMX on M-series Apple Silicon.
// Q: [b × dim] row-major f32 (pre-normalized queries)
// M: [n × dim] row-major f32 (pre-normalized corpus, contiguous)
// Output: scores [b × n], then top-k per row via binary heap.

#[cfg(target_os = "macos")]
mod accel {
    #[link(name = "Accelerate", kind = "framework")]
    extern "C" {
        pub fn cblas_sgemm(
            order: i32, transa: i32, transb: i32,
            m: i32, n: i32, k: i32,
            alpha: f32,
            a: *const f32, lda: i32,
            b: *const f32, ldb: i32,
            beta: f32,
            c: *mut f32, ldc: i32,
        );
    }
    pub const ROW_MAJOR: i32 = 101;
    pub const NO_TRANS: i32 = 111;
    pub const TRANS: i32 = 112;
}

/// Batch brute-force kNN using a single GEMM: Q[b×d] × M^T[d×n] → scores[b×n].
///
/// On macOS uses cblas_sgemm (Accelerate → AMX coprocessor).
/// On Linux falls back to ndarray matmul (AVX/NEON auto-dispatched).
///
/// `queries`: b pre-normalized f32 slices, each of length `dim`.
/// `matrix_flat`: row-major f32, shape (n × dim), pre-normalized.
/// Returns b result vecs, each sorted best-first (index, cosine_score).
pub fn top_k_batch_gemm(
    queries: &[&[f32]],
    matrix_flat: &[f32],
    n: usize,
    dim: usize,
    k: usize,
) -> Vec<Vec<(usize, f32)>> {
    let b = queries.len();
    if b == 0 || n == 0 || k == 0 {
        return vec![vec![]; b];
    }
    let k = k.min(n);

    // Build contiguous query matrix Q [b × dim].
    let mut q_flat: Vec<f32> = Vec::with_capacity(b * dim);
    for q in queries {
        q_flat.extend_from_slice(q);
    }

    // scores [b × n]
    let mut scores = vec![0.0f32; b * n];

    #[cfg(target_os = "macos")]
    {
        // cblas_sgemm(RowMajor, NoTrans, Trans, b, n, dim, 1.0, Q, dim, M, dim, 0.0, C, n)
        unsafe {
            accel::cblas_sgemm(
                accel::ROW_MAJOR, accel::NO_TRANS, accel::TRANS,
                b as i32, n as i32, dim as i32,
                1.0,
                q_flat.as_ptr(), dim as i32,
                matrix_flat.as_ptr(), dim as i32,
                0.0,
                scores.as_mut_ptr(), n as i32,
            );
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        use ndarray::{ArrayView2, Array2};
        let q_mat = ArrayView2::from_shape((b, dim), &q_flat).expect("shape");
        let m_mat = ArrayView2::from_shape((n, dim), matrix_flat).expect("shape");
        let result = q_mat.dot(&m_mat.t());
        scores.copy_from_slice(result.as_slice().expect("contiguous"));
    }

    // Top-k per row using a min-heap.
    scores.chunks_exact(n).map(|row| {
        let out = partial_top_k(
            row.iter().enumerate().map(|(i, &s)| (i, s)).collect(),
            k,
        );
        out
    }).collect()
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
