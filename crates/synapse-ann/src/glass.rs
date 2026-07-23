//! Glass-style SIMD beam-search HNSW scaffold.
//!
//! Inspired by Zilliz Glass (2024): cache-friendly CSR graph layout,
//! SIMD-batched distance compute over 16-candidate frontier batches,
//! beam-search (multi-frontier heap) instead of greedy-HNSW.
//!
//! Feature gate: `glass-backend` (default off).
//!
//! Current scope: full API surface + CSR graph layout + sequential
//! reference beam-search. SIMD distance batch = TODO (see `dist_batch`).

use std::collections::BinaryHeap;
use std::path::Path;

use crate::{AnnError, AnnIndex, SearchResults};

// ── types ────────────────────────────────────────────────────────────────────

pub type DocId = u64;

/// Heap entry: min-heap by distance (negate for std BinaryHeap max-heap).
#[derive(Clone, Copy, PartialEq)]
struct HeapEntry {
    neg_dist: f32,
    id: u32, // internal index
}
impl Eq for HeapEntry {}
impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.neg_dist
            .partial_cmp(&other.neg_dist)
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}

// ── CSR adjacency ────────────────────────────────────────────────────────────

/// Compressed-sparse-row adjacency list.
/// `offsets[i]..offsets[i+1]` = neighbour range in `neighbours`.
struct CsrGraph {
    offsets: Vec<u32>,
    neighbours: Vec<u32>,
}

impl CsrGraph {
    fn new(n: usize) -> Self {
        Self {
            offsets: vec![0u32; n + 1],
            neighbours: Vec::new(),
        }
    }

    fn neighbours(&self, i: usize) -> &[u32] {
        let s = self.offsets[i] as usize;
        let e = self.offsets[i + 1] as usize;
        &self.neighbours[s..e]
    }
}

// ── GlassIndex ───────────────────────────────────────────────────────────────

/// Glass HNSW index (single-layer, layer-0 only for now).
///
/// Build via [`GlassIndex::build`], search via [`GlassIndex::search_beam_simd`].
pub struct GlassIndex {
    /// Flat f32 vector store: `vecs[i * dim .. (i+1) * dim]`.
    vecs: Vec<f32>,
    /// External ids parallel to `vecs`.
    ids: Vec<DocId>,
    /// CSR adjacency for layer-0 graph.
    graph: CsrGraph,
    dim: usize,
    m: usize,
    ef_construction: usize,
    entry: u32,
}

// ── distance kernel ───────────────────────────────────────────────────────────

/// Scalar L2² distance (reference path).
/// TODO: swap to synapse-kernel NEON dot / f16 kernel when dim is known at
/// compile time and `target_feature = "neon"` is set.
#[inline(always)]
fn l2sq(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| {
            let d = x - y;
            d * d
        })
        .sum()
}

/// Batch distance: compute dist(query, vecs[ids[i]]) for all i in `batch`.
/// Returns distances in the same order as `batch`.
///
/// TODO: replace inner loop with SIMD intrinsics (ARM NEON / x86 AVX2).
/// Pattern: load 4×f32 lanes for 4 candidates simultaneously, accumulate
/// diff² across dim with `vld1q_f32` / `vmulq_f32` / `vaddq_f32`.
/// Current impl is the scalar reference (equivalent semantics, ~4× slower).
fn dist_batch(query: &[f32], vecs: &[f32], dim: usize, batch: &[u32]) -> Vec<f32> {
    batch
        .iter()
        .map(|&i| {
            let base = i as usize * dim;
            l2sq(query, &vecs[base..base + dim])
        })
        .collect()
}

// ── build ─────────────────────────────────────────────────────────────────────

impl GlassIndex {
    /// Build a Glass index from a flat slice of row-major f32 vectors.
    ///
    /// `vecs` — `n * dim` f32 values.
    /// `m`    — max neighbours per node (HNSW M param, typical 16–32).
    /// `ef_c` — ef_construction: beam width during build (typical 100–400).
    pub fn build(vecs: &[Vec<f32>], m: usize, ef_c: usize) -> Result<Self, AnnError> {
        if vecs.is_empty() {
            return Ok(Self {
                vecs: Vec::new(),
                ids: Vec::new(),
                graph: CsrGraph::new(0),
                dim: 0,
                m,
                ef_construction: ef_c,
                entry: 0,
            });
        }

        let dim = vecs[0].len();
        let n = vecs.len();

        // Flat vector store.
        let mut flat: Vec<f32> = Vec::with_capacity(n * dim);
        for v in vecs {
            if v.len() != dim {
                return Err(AnnError::DimMismatch {
                    expected: dim,
                    actual: v.len(),
                });
            }
            flat.extend_from_slice(v);
        }

        let ids: Vec<DocId> = (0..n as DocId).collect();

        // Build layer-0 graph via greedy insertion + beam-search.
        // Each node gets up to `m` neighbours (bi-directional, capped).
        let mut adj: Vec<Vec<u32>> = vec![Vec::new(); n];

        for i in 1..n {
            let q = &flat[i * dim..(i + 1) * dim];
            // Beam-search the already-inserted nodes for `m` closest.
            let cands = beam_search_internal(&flat, dim, &adj, 0, q, m, ef_c.min(i));
            for (nb, _) in &cands {
                let nb = *nb as usize;
                if adj[i].len() < m {
                    adj[i].push(nb as u32);
                }
                if adj[nb].len() < m {
                    adj[nb].push(i as u32);
                }
            }
        }

        // Compact into CSR.
        let mut graph = CsrGraph::new(n);
        let mut off: u32 = 0;
        for (i, nbrs) in adj.iter().enumerate() {
            graph.offsets[i] = off;
            graph.neighbours.extend_from_slice(nbrs);
            off += nbrs.len() as u32;
        }
        graph.offsets[n] = off;

        Ok(Self {
            vecs: flat,
            ids,
            graph,
            dim,
            m,
            ef_construction: ef_c,
            entry: 0,
        })
    }
}

// ── internal beam-search ──────────────────────────────────────────────────────

/// Concrete beam-search over dynamic adjacency (used during build).
///
/// `adj[i]` = current neighbour list for node i (grows during insertion).
/// Only looks at nodes 0..n_built.
fn beam_search_internal(
    flat: &[f32],
    dim: usize,
    adj: &[Vec<u32>],
    entry: u32,
    query: &[f32],
    k: usize,
    ef: usize,
) -> Vec<(u32, f32)> {
    let n = adj.len();
    if n == 0 {
        return Vec::new();
    }

    // visited bitset (simple vec<bool> — OK for scaffold).
    let mut visited = vec![false; n];

    // `candidates`: min-heap by distance (nodes to expand).
    // `results`: max-heap by distance (top-ef found so far).
    let mut candidates: BinaryHeap<HeapEntry> = BinaryHeap::new();
    let mut results: BinaryHeap<HeapEntry> = BinaryHeap::new(); // max-heap

    let entry = entry as usize;
    let d0 = l2sq(query, &flat[entry * dim..(entry + 1) * dim]);
    visited[entry] = true;
    candidates.push(HeapEntry {
        neg_dist: -d0,
        id: entry as u32,
    });
    results.push(HeapEntry {
        neg_dist: -d0,
        id: entry as u32,
    });

    while let Some(cur) = candidates.pop() {
        let worst_result = results.peek().map(|e| -e.neg_dist).unwrap_or(f32::MAX);
        if -cur.neg_dist > worst_result && results.len() >= ef {
            break;
        }

        // Collect neighbours into a batch for SIMD distance.
        let nbrs = &adj[cur.id as usize];
        let batch: Vec<u32> = nbrs
            .iter()
            .copied()
            .filter(|&nb| {
                let nb = nb as usize;
                nb < n && !visited[nb]
            })
            .collect();

        // Mark visited before compute to avoid duplicate processing.
        for &nb in &batch {
            visited[nb as usize] = true;
        }

        // SIMD-batched distance (scalar reference, see dist_batch TODO).
        let dists = dist_batch(query, flat, dim, &batch);

        for (&nb, d) in batch.iter().zip(dists) {
            if results.len() < ef || d < worst_result {
                candidates.push(HeapEntry {
                    neg_dist: -d,
                    id: nb,
                });
                results.push(HeapEntry {
                    neg_dist: -d,
                    id: nb,
                });
                if results.len() > ef {
                    results.pop();
                }
            }
        }
    }

    // Drain results into sorted vec, keep top-k.
    let mut out: Vec<(u32, f32)> = results.into_iter().map(|e| (e.id, -e.neg_dist)).collect();
    out.sort_unstable_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(k);
    out
}

/// Beam-search over a built CSR graph (query path).
fn beam_search_csr(
    flat: &[f32],
    dim: usize,
    graph: &CsrGraph,
    entry: u32,
    query: &[f32],
    k: usize,
    ef: usize,
) -> Vec<(u32, f32)> {
    let n = graph.offsets.len().saturating_sub(1);
    if n == 0 {
        return Vec::new();
    }

    let mut visited = vec![false; n];
    let mut candidates: BinaryHeap<HeapEntry> = BinaryHeap::new();
    let mut results: BinaryHeap<HeapEntry> = BinaryHeap::new();

    let entry = entry as usize;
    let d0 = l2sq(query, &flat[entry * dim..(entry + 1) * dim]);
    visited[entry] = true;
    candidates.push(HeapEntry {
        neg_dist: -d0,
        id: entry as u32,
    });
    results.push(HeapEntry {
        neg_dist: -d0,
        id: entry as u32,
    });

    while let Some(cur) = candidates.pop() {
        let worst = results.peek().map(|e| -e.neg_dist).unwrap_or(f32::MAX);
        if -cur.neg_dist > worst && results.len() >= ef {
            break;
        }

        let nbrs = graph.neighbours(cur.id as usize);
        let batch: Vec<u32> = nbrs
            .iter()
            .copied()
            .filter(|&nb| !visited[nb as usize])
            .collect();
        for &nb in &batch {
            visited[nb as usize] = true;
        }

        let dists = dist_batch(query, flat, dim, &batch);
        for (&nb, d) in batch.iter().zip(dists) {
            if results.len() < ef || d < worst {
                candidates.push(HeapEntry {
                    neg_dist: -d,
                    id: nb,
                });
                results.push(HeapEntry {
                    neg_dist: -d,
                    id: nb,
                });
                if results.len() > ef {
                    results.pop();
                }
            }
        }
    }

    let mut out: Vec<(u32, f32)> = results.into_iter().map(|e| (e.id, -e.neg_dist)).collect();
    out.sort_unstable_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(k);
    out
}

impl GlassIndex {
    /// Beam-search with SIMD-batched distance (Glass-style).
    /// `ef` — beam width (≥ k). Returns top-k (DocId, distance) ascending.
    pub fn search_beam_simd(
        &self,
        query: &[f32],
        k: usize,
        ef: usize,
    ) -> Result<SearchResults, AnnError> {
        if self.vecs.is_empty() {
            return Ok(Vec::new());
        }
        if query.len() != self.dim {
            return Err(AnnError::DimMismatch {
                expected: self.dim,
                actual: query.len(),
            });
        }
        let ef = ef.max(k);
        let results = beam_search_csr(&self.vecs, self.dim, &self.graph, self.entry, query, k, ef);
        Ok(results
            .into_iter()
            .map(|(i, d)| (self.ids[i as usize], d))
            .collect())
    }
}

// ── AnnIndex impl ─────────────────────────────────────────────────────────────

impl AnnIndex for GlassIndex {
    fn insert(&mut self, _id: u64, _vector: &[f32]) -> Result<(), AnnError> {
        // Dynamic insertion not yet implemented; rebuild via `build()`.
        Err(AnnError::Other(
            "GlassIndex: dynamic insert not supported — use build()".into(),
        ))
    }

    fn remove(&mut self, _id: u64) -> Result<usize, AnnError> {
        Err(AnnError::Other("GlassIndex: remove not supported".into()))
    }

    fn search(&self, query: &[f32], k: usize) -> Result<SearchResults, AnnError> {
        self.search_beam_simd(query, k, k * 2)
    }

    fn len(&self) -> usize {
        self.ids.len()
    }

    fn save(&self, _path: &Path) -> Result<(), AnnError> {
        // TODO: bincode / rkyv serialization of (vecs, ids, graph).
        Err(AnnError::Other(
            "GlassIndex: save not yet implemented".into(),
        ))
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_vecs(n: usize, dim: usize) -> Vec<Vec<f32>> {
        (0..n)
            .map(|i| (0..dim).map(|j| (i * dim + j) as f32 * 0.01).collect())
            .collect()
    }

    #[test]
    fn smoke_build_search_1k() {
        let n = 1_000;
        let dim = 32;
        let vecs = make_vecs(n, dim);

        let idx = GlassIndex::build(&vecs, 16, 64).expect("build failed");
        assert_eq!(idx.len(), n);

        // Search with the first vector as query — expect id=0 as nearest.
        let query = &vecs[0];
        let results = idx.search_beam_simd(query, 10, 40).expect("search failed");

        assert!(!results.is_empty(), "search returned empty");
        assert!(results.len() <= 10);
        // Nearest neighbour of vec[0] must be itself (distance ≈ 0).
        let (top_id, top_dist) = results[0];
        assert_eq!(top_id, 0, "expected self as nearest, got {top_id}");
        assert!(
            top_dist < 1e-6,
            "self-distance should be ~0, got {top_dist}"
        );
    }

    #[test]
    fn smoke_build_empty() {
        let idx = GlassIndex::build(&[], 16, 64).expect("build failed on empty");
        assert!(idx.is_empty());
        let res = idx.search_beam_simd(&[], 5, 10).expect("search on empty");
        assert!(res.is_empty());
    }

    #[test]
    fn dim_mismatch_returns_error() {
        let vecs = make_vecs(10, 8);
        let idx = GlassIndex::build(&vecs, 4, 20).unwrap();
        let bad_query = vec![0.0f32; 16]; // wrong dim
        assert!(matches!(
            idx.search(&bad_query, 3),
            Err(AnnError::DimMismatch { .. })
        ));
    }
}
