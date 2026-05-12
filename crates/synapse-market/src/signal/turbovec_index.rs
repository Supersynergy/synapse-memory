use std::path::Path;

use turbovec::IdMapIndex;

use super::SignalId;
use crate::error::{Error, Result};

/// TurboQuant ANN index wrapping `turbovec::IdMapIndex`.
///
/// Uses 4-bit quantisation (best recall/speed balance at 768d).
/// No training required — data-oblivious unlike IVF-based indexes.
/// Recommended default for ≤10M 768-d vectors on memory-constrained hosts.
pub struct TurboVecIndex {
    inner: IdMapIndex,
    dim: usize,
}

impl TurboVecIndex {
    /// Build from `(id, 768-d f32)` pairs.
    /// `bit_width` 2–4 (4 = best recall, 2 = most compact).
    pub fn build(entries: &[(SignalId, Vec<f32>)], bit_width: usize) -> Result<Self> {
        if entries.is_empty() {
            return Err(Error::Market("empty entries".into()));
        }
        let dim = entries[0].1.len();
        if dim % 8 != 0 {
            return Err(Error::Market(format!("dim {dim} not a multiple of 8")));
        }
        let mut inner = IdMapIndex::new(dim, bit_width);
        let flat: Vec<f32> = entries
            .iter()
            .flat_map(|(_, v)| v.iter().copied())
            .collect();
        let ids: Vec<u64> = entries.iter().map(|(id, _)| *id).collect();
        inner.add_with_ids(&flat, &ids);
        inner.prepare();
        Ok(Self { inner, dim })
    }

    /// Search for top-K nearest by inner-product (cosine on pre-normalised vecs).
    pub fn search(&self, query: &[f32], top_k: usize) -> Result<Vec<(SignalId, f32)>> {
        if query.len() != self.dim {
            return Err(Error::Market(format!(
                "query dim {} != index dim {}",
                query.len(),
                self.dim
            )));
        }
        let (scores, ids) = self.inner.search(query, top_k);
        Ok(ids.into_iter().zip(scores).collect())
    }

    /// Bytes per vector (packed codes + norm f32).
    pub fn bytes_per_vec(&self) -> usize {
        self.inner.dim() * self.inner.bit_width() / 8 + 4
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Serialize to path (writes a `.tvim` file).
    pub fn save(&self, path: &Path) -> Result<()> {
        self.inner
            .write(path)
            .map_err(|e| Error::Market(e.to_string()))
    }

    /// Load from a `.tvim` file written by [`Self::save`].
    pub fn load(path: &Path) -> Result<Self> {
        let inner = IdMapIndex::load(path).map_err(|e| Error::Market(e.to_string()))?;
        let dim = inner.dim();
        Ok(Self { inner, dim })
    }
}
