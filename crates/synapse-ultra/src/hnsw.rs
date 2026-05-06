//! HNSW index using usearch (C++ bindings). Feature-gated: `hnsw`.
//! Lazy-built at snapshot load; persisted at ~/.synapse/ultra_hnsw.usearch.

#[cfg(feature = "hnsw")]
mod inner {
    use std::path::Path;

    use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

    use crate::error::{Result, UltraError};
    use crate::snapshot::EMBED_DIM;

    fn make_options() -> IndexOptions {
        let mut opts = IndexOptions::default();
        opts.dimensions = EMBED_DIM;
        opts.metric = MetricKind::Cos;
        opts.quantization = ScalarKind::F32;
        opts.connectivity = 16; // M
        opts.expansion_add = 128; // ef_construction
        opts
    }

    pub fn build(
        matrix_f32: &ndarray::Array2<f32>,
        ids: &[i64],
        hnsw_path: &Path,
        snap_mtime: u64,
    ) -> Result<Index> {
        // Check if saved index is fresh
        if let Some(idx) = try_load(hnsw_path, ids.len(), snap_mtime) {
            tracing::info!("loaded HNSW index from {:?}", hnsw_path);
            return Ok(idx);
        }

        tracing::info!("building HNSW index for {} vectors (M=16, ef=128)", ids.len());
        let t0 = std::time::Instant::now();

        let opts = make_options();
        let index = Index::new(&opts).map_err(|e| UltraError::Anyhow(anyhow::anyhow!("usearch: {e}")))?;
        index.reserve(ids.len()).map_err(|e| UltraError::Anyhow(anyhow::anyhow!("reserve: {e}")))?;

        // Add all vectors — usearch uses u64 keys; we store offset (0..n), resolve id separately
        let n = ids.len();
        for i in 0..n {
            let row = matrix_f32.row(i);
            let slice: &[f32] = row.as_slice().expect("contiguous row");
            index.add(i as u64, slice)
                .map_err(|e| UltraError::Anyhow(anyhow::anyhow!("add[{i}]: {e}")))?;
        }

        let elapsed = t0.elapsed();
        tracing::info!("HNSW build done in {:.2}s", elapsed.as_secs_f64());
        if elapsed.as_secs() > 10 {
            tracing::warn!("HNSW build exceeded 10s — consider pre-caching");
        }

        // Persist
        if let Some(parent) = hnsw_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Write mtime tag alongside: hnsw_path + ".mtime"
        let _ = std::fs::write(
            hnsw_path.with_extension("usearch.mtime"),
            snap_mtime.to_le_bytes(),
        );
        index.save(hnsw_path.to_str().unwrap())
            .map_err(|e| UltraError::Anyhow(anyhow::anyhow!("save: {e}")))?;

        Ok(index)
    }

    fn try_load(hnsw_path: &Path, expected_len: usize, snap_mtime: u64) -> Option<Index> {
        // Check mtime tag
        let mtime_path = hnsw_path.with_extension("usearch.mtime");
        let stored_mtime = std::fs::read(&mtime_path).ok()
            .and_then(|b| b.try_into().ok().map(u64::from_le_bytes));
        if stored_mtime != Some(snap_mtime) {
            return None;
        }
        let opts = make_options();
        let index = Index::new(&opts).ok()?;
        index.load(hnsw_path.to_str()?).ok()?;
        if index.size() != expected_len {
            return None;
        }
        Some(index)
    }

    pub fn search(
        index: &Index,
        ids: &[i64],
        query: &[f32],
        k: usize,
        ef: usize,
    ) -> Vec<(usize, f32)> {
        let results = match index.search(query, k * 2) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("hnsw search: {e}");
                return vec![];
            }
        };
        let _ = ef; // ef is set per-index globally; usearch 2.x sets ef via env or index opts

        let n = ids.len();
        results.keys.iter().zip(results.distances.iter())
            .filter(|(&key, _)| (key as usize) < n)
            .map(|(&key, &dist)| (key as usize, 1.0 - dist)) // cosine dist → similarity
            .collect()
    }
}

#[cfg(feature = "hnsw")]
pub use inner::{build, search};

// Stub types when feature disabled
#[cfg(not(feature = "hnsw"))]
pub struct HnswPlaceholder;
