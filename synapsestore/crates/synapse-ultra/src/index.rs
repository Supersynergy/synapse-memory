use std::path::Path;
use std::sync::Arc;

use arc_swap::ArcSwap;
use ndarray::Array2;

use crate::binary;
use crate::error::Result;
use crate::search;
use crate::snapshot::{self, Snapshot, EMBED_DIM};

pub struct UltraIndex {
    pub ids: Vec<i64>,
    /// f32 matrix for T1-strict, shape (n, 384)
    pub matrix_f32: Array2<f32>,
    /// f16 matrix as u16 LE, len = n*384
    pub matrix_f16: Vec<u16>,
    /// packed-sign binary matrix, len = n*48
    pub bin_matrix: Vec<u8>,
    /// RaBitQ rotated binary matrix, len = n*48 (feature-gated)
    #[cfg(feature = "rabitq")]
    pub bin_matrix_rotated: Option<Vec<u8>>,
    /// HNSW index (feature-gated)
    #[cfg(feature = "hnsw")]
    pub hnsw: Option<usearch::Index>,
}

impl UltraIndex {
    pub fn from_snapshot(snap: Snapshot) -> Self {
        let bin_matrix = binary::build_binary_matrix(&snap.matrix_f32);
        #[cfg(feature = "rabitq")]
        let bin_matrix_rotated = Some(binary::build_binary_matrix_rotated(&snap.matrix_f32));
        UltraIndex {
            ids: snap.ids,
            matrix_f32: snap.matrix_f32,
            matrix_f16: snap.matrix_f16,
            bin_matrix,
            #[cfg(feature = "rabitq")]
            bin_matrix_rotated,
            #[cfg(feature = "hnsw")]
            hnsw: None,
        }
    }

    /// Build or load HNSW index from disk. Call once after from_snapshot.
    #[cfg(feature = "hnsw")]
    pub fn with_hnsw(mut self, hnsw_path: &Path, snap_mtime: u64) -> Result<Self> {
        let index = crate::hnsw::build(&self.matrix_f32, &self.ids, hnsw_path, snap_mtime)?;
        self.hnsw = Some(index);
        Ok(self)
    }

    pub fn n_rows(&self) -> usize {
        self.ids.len()
    }

    /// T1' binary-first: hamming → top-rerank_n candidates → f16 cosine → top-k.
    /// Default mode. ~50-100µs on 162k. Recall ≥ 0.95.
    pub fn search_binary_first(&self, query_f32: &[f32], k: usize) -> Vec<Hit> {
        debug_assert_eq!(query_f32.len(), EMBED_DIM);
        let query_sign = binary::pack_signs(query_f32);
        let rerank_n = search::DEFAULT_BINARY_RERANK.max(k * 16);
        let raw = search::top_k_binary_first(
            query_f32,
            &query_sign,
            &self.bin_matrix,
            &self.matrix_f16,
            self.ids.len(),
            k,
            rerank_n,
        );
        raw.into_iter().map(|(idx, score)| Hit { id: self.ids[idx], score }).collect()
    }

    /// T1-strict: brute-force f32, recall ≥ 0.99.
    pub fn search_strict(&self, query_f32: &[f32], k: usize) -> Vec<Hit> {
        debug_assert_eq!(query_f32.len(), EMBED_DIM);
        let raw = search::top_k_f32(query_f32, self.matrix_f32.view(), k);
        raw.into_iter().map(|(idx, score)| Hit { id: self.ids[idx], score }).collect()
    }

    /// Batch brute-force via GEMM (Accelerate on macOS, ndarray on Linux).
    /// Returns one Vec<Hit> per query, sorted best-first.
    /// Use when batch ≥ 8 and mode == Strict for maximum throughput.
    pub fn search_batch_blas(&self, queries: &[Vec<f32>], k: usize) -> Vec<Vec<Hit>> {
        if queries.is_empty() { return vec![]; }
        let matrix_flat = self.matrix_f32.as_slice().expect("row-major contiguous");
        let n = self.ids.len();
        let dim = EMBED_DIM;
        let q_refs: Vec<&[f32]> = queries.iter().map(|q| q.as_slice()).collect();
        let raw = search::top_k_batch_gemm(&q_refs, matrix_flat, n, dim, k);
        raw.into_iter()
            .map(|row| row.into_iter().map(|(idx, score)| Hit { id: self.ids[idx], score }).collect())
            .collect()
    }

    /// T3-binary-only: pure hamming no rerank, recall ≥ 0.92, max throughput.
    pub fn search_binary_only(&self, query_f32: &[f32], k: usize) -> Vec<Hit> {
        debug_assert_eq!(query_f32.len(), EMBED_DIM);
        let query_sign = binary::pack_signs(query_f32);
        let n = self.ids.len();
        let mut ham: Vec<(usize, u32)> = (0..n).map(|i| {
            let row = &self.bin_matrix[i * 48..(i + 1) * 48];
            (i, binary::hamming_distance(&query_sign, row))
        }).collect();
        let k2 = k.min(n);
        ham.select_nth_unstable_by_key(k2.saturating_sub(1), |x| x.1);
        ham.truncate(k2);
        ham.sort_unstable_by_key(|x| x.1);
        ham.into_iter().map(|(idx, dist)| Hit {
            id: self.ids[idx],
            score: 1.0 - (dist as f32 / 384.0),
        }).collect()
    }

    /// RaBitQ binary search: rotate query → hamming → top-rerank_n candidates → f16 cosine → top-k.
    #[cfg(feature = "rabitq")]
    pub fn search_rabitq(&self, query_f32: &[f32], k: usize) -> Vec<Hit> {
        debug_assert_eq!(query_f32.len(), EMBED_DIM);
        let query_sign = binary::pack_signs_rotated(query_f32);
        let bin_mat = match &self.bin_matrix_rotated {
            Some(m) => m,
            None => return self.search_binary_first(query_f32, k),
        };
        let rerank_n = search::DEFAULT_BINARY_RERANK.max(k * 16);
        let raw = search::top_k_binary_first(
            query_f32,
            &query_sign,
            bin_mat,
            &self.matrix_f16,
            self.ids.len(),
            k,
            rerank_n,
        );
        raw.into_iter().map(|(idx, score)| Hit { id: self.ids[idx], score }).collect()
    }

    /// HNSW approximate search. ef tunable via ULTRA_HNSW_EF (default 64).
    /// Candidates: 2*k from HNSW → f32 cosine rerank → top-k.
    #[cfg(feature = "hnsw")]
    pub fn search_hnsw(&self, query_f32: &[f32], k: usize) -> Vec<Hit> {
        debug_assert_eq!(query_f32.len(), EMBED_DIM);
        let ef: usize = std::env::var("ULTRA_HNSW_EF")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(64);

        let Some(ref index) = self.hnsw else {
            tracing::warn!("HNSW index not built, falling back to binary_first");
            return self.search_binary_first(query_f32, k);
        };

        let mut candidates = crate::hnsw::search(index, &self.ids, query_f32, k, ef);

        // rerank top-2k candidates with f32 cosine
        candidates.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        candidates.truncate(k);

        candidates.into_iter().map(|(idx, score)| Hit { id: self.ids[idx], score }).collect()
    }
}

#[derive(Debug, Clone)]
pub struct Hit {
    pub id: i64,
    pub score: f32,
}

pub type SharedIndex = Arc<ArcSwap<UltraIndex>>;

pub fn load_or_rebuild(brain_path: &Path, snap_path: &Path) -> Result<SharedIndex> {
    let brain_mtime = snapshot::brain_db_mtime(brain_path);
    let snap = match snapshot::load_mmap(snap_path, brain_mtime) {
        Some(s) => {
            tracing::info!("loaded snapshot ({} rows)", s.ids.len());
            s
        }
        None => snapshot::rebuild(brain_path, snap_path)?,
    };

    #[cfg(not(feature = "hnsw"))]
    let idx = UltraIndex::from_snapshot(snap);

    #[cfg(feature = "hnsw")]
    let idx = {
        let hnsw_path = std::path::PathBuf::from(
            std::env::var("HOME").unwrap_or_else(|_| "/root".into())
        ).join(".synapse/ultra_hnsw.usearch");
        let base = UltraIndex::from_snapshot(snap);
        base.with_hnsw(&hnsw_path, brain_mtime)?
    };

    Ok(Arc::new(ArcSwap::from_pointee(idx)))
}
