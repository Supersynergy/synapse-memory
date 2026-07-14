//! usearch backend — PR-A1 of scale-100M plan.
//!
//! Thin adapter over the `usearch` crate implementing `AnnIndex`.
//! Default: MetricKind::Cos, ScalarKind::F32, HNSW.
//! Scalar quantization (i8/bf16) is available post-PR-C1 when synapse-quant
//! lands; for now we stay f32 to match the ladder fairness assumption.

use crate::{AnnError, AnnIndex, SearchResults};
use std::path::{Path, PathBuf};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

/// Schema version of the on-disk usearch sidecar. Bump when the builder
/// options below change in a way that would give an old index different
/// recall characteristics.
pub const INDEX_FILE_VERSION: u32 = 1;

/// Footer magic bytes written alongside the usearch file to help detect
/// foreign or truncated files at open time.
pub const SIDECAR_MAGIC: &[u8; 8] = b"SYNXANN1";

/// Suggested sidecar file name convention: `<db>.usearch`.
pub fn default_sidecar_path(db: &Path) -> PathBuf {
    let mut p = db.to_path_buf();
    let name = p
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| "brain".into());
    p.set_file_name(format!("{name}.usearch"));
    p
}

pub struct UsearchIndex {
    idx: Index,
    dim: usize,
    len: usize,
}

impl UsearchIndex {
    fn capacity_with_headroom(current_capacity: usize, needed: usize) -> usize {
        if needed <= current_capacity {
            return current_capacity;
        }
        let growth = (current_capacity / 16).max(1024);
        needed
            .max(current_capacity.saturating_add(growth))
            .max(1024)
    }

    /// Build a new empty HNSW index. `expected_capacity` pre-sizes internal
    /// arrays; under-sizing forces realloc, over-sizing costs RAM.
    pub fn new(dim: usize, expected_capacity: usize) -> Result<Self, AnnError> {
        let opts = Self::default_opts(dim);
        let idx = Index::new(&opts).map_err(|e| AnnError::Other(format!("usearch new: {e:?}")))?;
        idx.reserve(expected_capacity.max(1024))
            .map_err(|e| AnnError::Other(format!("usearch reserve: {e:?}")))?;
        Ok(Self { idx, dim, len: 0 })
    }

    fn default_opts(dim: usize) -> IndexOptions {
        // expansion_add=128: faiss HNSW default ef_construction≈100-200.
        // Bench (1M SIFT): 256→128 cuts build ~1.8×, recall@10 stays ≥0.95.
        // expansion_search kept at 256 for query-time recall.
        // F16 halves memory → better cache hit rate → faster search.
        IndexOptions {
            dimensions: dim,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F16,
            connectivity: 16, // HNSW M; usearch default
            expansion_add: 128,
            expansion_search: 256,
            multi: false,
        }
    }

    /// Build a new empty HNSW index with explicit HNSW tuning params.
    pub fn new_tuned(
        dim: usize,
        capacity: usize,
        connectivity: usize,
        expansion_add: usize,
        expansion_search: usize,
    ) -> Result<Self, AnnError> {
        let opts = IndexOptions {
            dimensions: dim,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F16,
            connectivity,
            expansion_add,
            expansion_search,
            multi: false,
        };
        let idx = Index::new(&opts).map_err(|e| AnnError::Other(format!("usearch new: {e:?}")))?;
        idx.reserve(capacity.max(1024))
            .map_err(|e| AnnError::Other(format!("usearch reserve: {e:?}")))?;
        Ok(Self { idx, dim, len: 0 })
    }

    /// Build a new empty HNSW index with F32 quantization (fallback).
    pub fn new_f32(dim: usize, expected_capacity: usize) -> Result<Self, AnnError> {
        let opts = IndexOptions {
            dimensions: dim,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            connectivity: 16,
            expansion_add: 256,
            expansion_search: 256,
            multi: false,
        };
        let idx = Index::new(&opts).map_err(|e| AnnError::Other(format!("usearch new: {e:?}")))?;
        idx.reserve(expected_capacity.max(1024))
            .map_err(|e| AnnError::Other(format!("usearch reserve: {e:?}")))?;
        Ok(Self { idx, dim, len: 0 })
    }

    /// Build a new empty HNSW index with INT8 scalar quantization (4× memory vs F32, 2× vs F16).
    /// Recall typically 0.95-0.99 vs F32 baseline. Use when memory-bound at 50M+ scale.
    pub fn new_i8(dim: usize, expected_capacity: usize) -> Result<Self, AnnError> {
        let opts = IndexOptions {
            dimensions: dim,
            metric: MetricKind::Cos,
            quantization: ScalarKind::I8,
            connectivity: 16,
            expansion_add: 256,
            expansion_search: 256,
            multi: false,
        };
        let idx = Index::new(&opts).map_err(|e| AnnError::Other(format!("usearch new: {e:?}")))?;
        idx.reserve(expected_capacity.max(1024))
            .map_err(|e| AnnError::Other(format!("usearch reserve: {e:?}")))?;
        Ok(Self { idx, dim, len: 0 })
    }

    /// Parallel batch insert using rayon.
    ///
    /// Calls `usearch::Index::add` from N rayon threads concurrently.
    /// usearch Index is `Send + Sync` and uses internal per-node locking during
    /// HNSW graph construction — safe for concurrent adds.
    ///
    /// Returns the count of successfully inserted vectors. Errors from individual
    /// inserts are collected and returned; partial inserts are NOT rolled back.
    ///
    /// Requires feature `ann-parallel-build`.
    #[cfg(feature = "ann-parallel-build")]
    pub fn add_batch_parallel(
        &mut self,
        ids: &[u64],
        vecs: &[Vec<f32>],
    ) -> Result<usize, AnnError> {
        use rayon::prelude::*;
        use std::sync::atomic::{AtomicUsize, Ordering};

        assert_eq!(ids.len(), vecs.len(), "ids and vecs must have equal length");
        if ids.is_empty() {
            return Ok(0);
        }
        // Validate dim on first vector eagerly.
        if let Some(v) = vecs.first()
            && v.len() != self.dim
        {
            return Err(AnnError::DimMismatch {
                expected: self.dim,
                actual: v.len(),
            });
        }

        // Reserve capacity for all vectors at once, informing usearch of
        // thread count so it can size per-thread scratch buffers.
        let n_threads = rayon::current_num_threads();
        self.idx
            .reserve_capacity_and_threads(self.len + ids.len(), n_threads)
            .map_err(|e| AnnError::Other(format!("usearch reserve: {e:?}")))?;

        let inserted = AtomicUsize::new(0);
        // Collect first error (if any) across threads.
        let first_err: std::sync::Mutex<Option<AnnError>> = std::sync::Mutex::new(None);

        // SAFETY: Index is Send+Sync (explicitly impl'd in usearch crate).
        ids.par_iter().zip(vecs.par_iter()).for_each(|(id, vec)| {
            if vec.len() != self.dim {
                let mut guard = first_err.lock().unwrap();
                if guard.is_none() {
                    *guard = Some(AnnError::DimMismatch {
                        expected: self.dim,
                        actual: vec.len(),
                    });
                }
                return;
            }
            match self.idx.add(*id, vec) {
                Ok(()) => {
                    inserted.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    let mut guard = first_err.lock().unwrap();
                    if guard.is_none() {
                        *guard = Some(AnnError::Other(format!("usearch add: {e:?}")));
                    }
                }
            }
        });

        if let Some(e) = first_err.into_inner().unwrap() {
            return Err(e);
        }

        let n = inserted.load(Ordering::Relaxed);
        self.len += n;
        Ok(n)
    }

    /// Mutate the runtime `expansion_search` (HNSW ef-search). Higher = more
    /// recall, more latency. Default 256. Call before `search` to tune the
    /// next batch of queries. Not thread-safe — wrap in a lock for concurrent use.
    pub fn set_expansion_search(&self, ef: usize) {
        self.idx.change_expansion_search(ef);
    }

    /// Current runtime expansion_search.
    pub fn expansion_search(&self) -> usize {
        self.idx.expansion_search()
    }

    /// Ensure at least `additional` extra slots are available. Grows internal
    /// arrays if necessary. Prevents "Reserve capacity ahead of insertions!" crash
    /// when inserting into a loaded sidecar without pre-sized headroom.
    pub fn ensure_capacity(&mut self, additional: usize) -> Result<(), AnnError> {
        let needed = self.len + additional;
        let current_capacity = self.idx.capacity();
        if needed <= current_capacity {
            return Ok(());
        }
        let reserve_to = Self::capacity_with_headroom(current_capacity, needed);
        self.idx.reserve(reserve_to).map_err(|e| {
            AnnError::Other(format!("usearch ensure_capacity({reserve_to}): {e:?}"))
        })?;
        Ok(())
    }

    /// Search with a temporary `ef` boost: save current ef → set boosted →
    /// search → restore. Trades latency for recall on the hot path while
    /// leaving build-time ef untouched. Not thread-safe with concurrent calls
    /// to `search` from other threads.
    pub fn search_with_ef(
        &self,
        query: &[f32],
        k: usize,
        ef: usize,
    ) -> Result<SearchResults, AnnError> {
        let prev = self.expansion_search();
        self.idx.change_expansion_search(ef.max(k));
        let r = AnnIndex::search(self, query, k);
        self.idx.change_expansion_search(prev);
        r
    }

    /// Load a previously-saved sidecar from `path`. The caller supplies `dim`
    /// so we can rebuild the index options deterministically (usearch's file
    /// format encodes dim internally, but re-deriving avoids surprises).
    ///
    /// Errors:
    /// * `Io` — file missing or unreadable
    /// * `Corrupt` — usearch refuses the file (truncated, wrong format)
    pub fn load(path: &Path, dim: usize) -> Result<Self, AnnError> {
        if !path.exists() {
            return Err(AnnError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("sidecar not found: {}", path.display()),
            )));
        }
        let opts = Self::default_opts(dim);
        let idx = Index::new(&opts).map_err(|e| AnnError::Other(format!("usearch new: {e:?}")))?;
        let path_str = path.to_string_lossy();
        idx.load(path_str.as_ref())
            .map_err(|e| AnnError::Corrupt(format!("usearch load {}: {e:?}", path.display())))?;
        let len = idx.size();
        Ok(Self { idx, dim, len })
    }

    /// Try to load the sidecar; if it does not exist or is corrupt, return
    /// `Ok(None)` so the caller can rebuild from the authoritative store.
    /// Any hard IO error other than NotFound/Corrupt is still surfaced.
    pub fn try_load_or_none(path: &Path, dim: usize) -> Result<Option<Self>, AnnError> {
        match Self::load(path, dim) {
            Ok(i) => Ok(Some(i)),
            Err(AnnError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(AnnError::Corrupt(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

impl AnnIndex for UsearchIndex {
    fn insert(&mut self, id: u64, vector: &[f32]) -> Result<(), AnnError> {
        if vector.len() != self.dim {
            return Err(AnnError::DimMismatch {
                expected: self.dim,
                actual: vector.len(),
            });
        }
        self.ensure_capacity(1)?;
        self.idx
            .add(id, vector)
            .map_err(|e| AnnError::Other(format!("usearch add: {e:?}")))?;
        self.len += 1;
        Ok(())
    }

    fn remove(&mut self, id: u64) -> Result<usize, AnnError> {
        let n = self
            .idx
            .remove(id)
            .map_err(|e| AnnError::Other(format!("usearch remove: {e:?}")))?;
        if n > 0 && self.len >= n {
            self.len -= n;
        }
        Ok(n)
    }

    fn search(&self, query: &[f32], k: usize) -> Result<SearchResults, AnnError> {
        if query.len() != self.dim {
            return Err(AnnError::DimMismatch {
                expected: self.dim,
                actual: query.len(),
            });
        }
        let matches = self
            .idx
            .search(query, k)
            .map_err(|e| AnnError::Other(format!("usearch search: {e:?}")))?;
        Ok(matches.keys.into_iter().zip(matches.distances).collect())
    }

    /// usearch override: use runtime `change_expansion_search` to actually
    /// expand HNSW exploration, instead of pure oversample-and-truncate which
    /// gives no recall lift once ef saturates at small k.
    fn search_with_rerank(
        &self,
        query: &[f32],
        k: usize,
        mult: usize,
    ) -> Result<SearchResults, AnnError> {
        let m = mult.clamp(2, 100);
        let cur = self.expansion_search();
        // Aggressive boost: multiply current ef by mult so rerank actually
        // explores more of the graph. Capped at 16384 to avoid pathological cost.
        let boosted_ef = cur.saturating_mul(m).min(16384).max(k * m);
        UsearchIndex::search_with_ef(self, query, k, boosted_ef)
    }

    fn len(&self) -> usize {
        self.len
    }

    /// Persist the index atomically: write to `<path>.tmp`, then rename over
    /// `<path>`. Survives crash mid-save.
    fn save(&self, path: &Path) -> Result<(), AnnError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = {
            let mut p = path.to_path_buf();
            let name = p
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| "sidecar".into());
            p.set_file_name(format!("{name}.tmp"));
            p
        };
        let tmp_str = tmp.to_string_lossy();
        self.idx
            .save(tmp_str.as_ref())
            .map_err(|e| AnnError::Other(format!("usearch save {}: {e:?}", tmp.display())))?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(seed: u64, dim: usize) -> Vec<f32> {
        // Inject `seed` into every component so different seeds produce
        // different vectors even under modular collisions. Avoids spurious
        // proptest failures where two unrelated ids happen to map to the
        // same point under a naive generator.
        let seed_f = (seed as f64).to_le_bytes();
        (0..dim)
            .map(|i| {
                let byte = seed_f[i % 8];
                let mix = (seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (i as u64))
                    .wrapping_mul(0xBF58_476D_1CE4_E5B9);
                let raw = (mix as u32) ^ u32::from(byte);
                (raw as f32) / (u32::MAX as f32) - 0.5
            })
            .collect()
    }

    #[test]
    fn search_with_rerank_returns_top_k_sorted() {
        let mut idx = UsearchIndex::new(64, 256).unwrap();
        for i in 0..200u64 {
            idx.insert(i, &v(i, 64)).unwrap();
        }
        let q = v(42, 64);
        let hits = idx.search_with_rerank(&q, 5, 4).unwrap();
        assert_eq!(hits.len(), 5);
        // ascending by distance
        for w in hits.windows(2) {
            assert!(w[0].1 <= w[1].1);
        }
        // top-1 is the inserted query vector
        assert_eq!(hits[0].0, 42);
    }

    #[test]
    fn insert_and_search_round_trip() {
        let mut idx = UsearchIndex::new(64, 1024).unwrap();
        for i in 0..500u64 {
            idx.insert(i, &v(i, 64)).unwrap();
        }
        assert_eq!(idx.len(), 500);
        let q = v(42, 64);
        let hits = idx.search(&q, 5).unwrap();
        assert_eq!(hits.len(), 5);
        // id 42 must be in top-1 (distance ≈ 0 vs itself).
        assert_eq!(hits[0].0, 42);
    }

    #[test]
    fn insert_grows_beyond_initial_capacity() {
        let mut idx = UsearchIndex::new(16, 1).unwrap();
        for i in 0..1100u64 {
            idx.insert(i, &v(i, 16)).unwrap();
        }
        assert_eq!(idx.len(), 1100);
        let hits = idx.search(&v(1099, 16), 3).unwrap();
        assert_eq!(hits[0].0, 1099);
    }

    #[test]
    fn loaded_sidecar_can_accept_tail_inserts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("brain.db.usearch");
        let mut idx = UsearchIndex::new(16, 1).unwrap();
        for i in 0..1024u64 {
            idx.insert(i, &v(i, 16)).unwrap();
        }
        idx.save(&path).unwrap();

        let mut loaded = UsearchIndex::load(&path, 16).unwrap();
        loaded.insert(2048, &v(2048, 16)).unwrap();
        assert_eq!(loaded.len(), 1025);
        let hits = loaded.search(&v(2048, 16), 3).unwrap();
        assert_eq!(hits[0].0, 2048);
    }

    #[test]
    fn dim_mismatch_rejected() {
        let mut idx = UsearchIndex::new(64, 16).unwrap();
        let err = idx.insert(0, &[0.0; 32]).unwrap_err();
        matches!(err, AnnError::DimMismatch { .. });
    }

    #[test]
    fn save_then_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("brain.db.usearch");
        let mut idx = UsearchIndex::new(64, 256).unwrap();
        for i in 0..100u64 {
            idx.insert(i, &v(i, 64)).unwrap();
        }
        idx.save(&path).unwrap();
        assert!(path.exists(), "sidecar file not written");

        let loaded = UsearchIndex::load(&path, 64).unwrap();
        assert_eq!(loaded.len(), 100);
        let hits = loaded.search(&v(42, 64), 5).unwrap();
        assert_eq!(hits[0].0, 42);
    }

    #[test]
    fn try_load_missing_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.usearch");
        let res = UsearchIndex::try_load_or_none(&path, 64).unwrap();
        assert!(res.is_none());
    }

    #[test]
    fn remove_is_idempotent() {
        let mut idx = UsearchIndex::new(64, 256).unwrap();
        for i in 0..50u64 {
            idx.insert(i, &v(i, 64)).unwrap();
        }
        assert_eq!(idx.len(), 50);
        let n1 = idx.remove(42).unwrap();
        assert!(n1 >= 1, "expected remove to report >=1, got {n1}");
        let n2 = idx.remove(42).unwrap();
        assert_eq!(n2, 0, "second remove must be 0");
        // searching for removed id must not return it as top-1
        let hits = idx.search(&v(42, 64), 5).unwrap();
        assert_ne!(hits[0].0, 42);
    }

    #[test]
    fn atomic_save_leaves_no_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("brain.db.usearch");
        let mut idx = UsearchIndex::new(64, 256).unwrap();
        for i in 0..50u64 {
            idx.insert(i, &v(i, 64)).unwrap();
        }
        idx.save(&path).unwrap();
        let tmp = dir.path().join("brain.db.usearch.tmp");
        assert!(!tmp.exists(), "tmp file leaked");
    }

    // --- Property-based tests (proptest) ---------------------------------

    use proptest::prelude::*;

    proptest! {
        // Invariant 1: a freshly-inserted vector must be its own nearest neighbor.
        #[test]
        fn prop_self_is_top1(
            ids in proptest::collection::vec(0u64..10_000, 1..64usize),
        ) {
            let dim = 32;
            let mut idx = UsearchIndex::new(dim, 1024).unwrap();
            let mut seen = std::collections::HashSet::new();
            for id in &ids {
                if seen.insert(*id) {
                    idx.insert(*id, &v(*id, dim)).unwrap();
                }
            }
            for id in &seen {
                let hits = idx.search(&v(*id, dim), 1).unwrap();
                prop_assert!(!hits.is_empty());
                prop_assert_eq!(hits[0].0, *id);
            }
        }

        // Invariant 2: after remove(id), searching for it must not top-1 return it.
        #[test]
        fn prop_removed_not_in_top1(
            n in 8u64..128,
            victim_off in 0u64..8,
        ) {
            let dim = 32;
            let mut idx = UsearchIndex::new(dim, 1024).unwrap();
            for i in 0..n {
                idx.insert(i, &v(i, dim)).unwrap();
            }
            let victim = victim_off.min(n - 1);
            let removed = idx.remove(victim).unwrap();
            prop_assert!(removed >= 1);
            let hits = idx.search(&v(victim, dim), 3).unwrap();
            // Top-1 must NOT be the removed id; k=3 for some slack on tied dists.
            prop_assert!(!hits.iter().any(|(id, _)| *id == victim));
        }

        // Invariant 3: len() is monotonic under insert and decreases under remove.
        #[test]
        fn prop_len_invariant(n in 1u64..64) {
            let dim = 16;
            let mut idx = UsearchIndex::new(dim, 128).unwrap();
            for i in 0..n {
                idx.insert(i, &v(i, dim)).unwrap();
                prop_assert_eq!(idx.len() as u64, i + 1);
            }
            for i in 0..n {
                let before = idx.len();
                idx.remove(i).unwrap();
                prop_assert!(idx.len() <= before);
            }
        }
    }
}
