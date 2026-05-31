use std::path::Path;

use rabitq_rs::{IvfRabitqIndex, Metric, RotatorType, SearchParams};
use simsimd::SpatialSimilarity;

use super::SignalId;
use crate::error::{Error, Result};
use crate::{SignalEntry, SignalHit};

// ── RaBitQ IVF index ────────────────────────────────────────────────────────

pub struct RabitqSignalIndex {
    inner: IvfRabitqIndex,
    ids: Vec<SignalId>,
}

impl RabitqSignalIndex {
    /// Build from a slice of (id, 768-d f32 vector) pairs.
    /// `n_clusters` ~ sqrt(N) is a good default.
    pub fn build(entries: &[SignalEntry], n_clusters: usize) -> Result<Self> {
        if entries.is_empty() {
            return Err(Error::Market("empty entries".into()));
        }
        let ids: Vec<SignalId> = entries.iter().map(|(id, _)| *id).collect();
        let vecs: Vec<Vec<f32>> = entries.iter().map(|(_, v)| v.clone()).collect();

        let inner = IvfRabitqIndex::train(
            &vecs,
            n_clusters,
            4,                    // 4-bit quantisation — balances recall vs speed
            Metric::InnerProduct, // cosine on pre-normalised vecs ≡ inner product
            RotatorType::FhtKacRotator,
            42,
            false,
        )
        .map_err(|e| Error::Market(e.to_string()))?;

        Ok(Self { inner, ids })
    }

    pub fn search(&self, query: &[f32], top_k: usize) -> Result<Vec<SignalHit>> {
        // nprobe = 20% of clusters gives good recall/speed trade-off for 768d vecs
        let nprobe = ((self.inner.cluster_count() / 5).max(1)).min(self.inner.cluster_count());
        let params = SearchParams::new(top_k, nprobe);
        let results = self
            .inner
            .search(query, params)
            .map_err(|e| Error::Market(e.to_string()))?;
        Ok(results
            .into_iter()
            .map(|r| (self.ids[r.id], r.score))
            .collect())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.inner
            .save_to_path(path)
            .map_err(|e| Error::Market(e.to_string()))?;
        // Save ids alongside: path + ".ids"
        let ids_path = path.with_extension("ids.bin");
        let bytes: Vec<u8> = self.ids.iter().flat_map(|id| id.to_le_bytes()).collect();
        std::fs::write(ids_path, bytes)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let inner =
            IvfRabitqIndex::load_from_path(path).map_err(|e| Error::Market(e.to_string()))?;
        let ids_path = path.with_extension("ids.bin");
        let bytes = std::fs::read(ids_path)?;
        let ids: Vec<SignalId> = bytes
            .chunks_exact(8)
            .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
            .collect();
        Ok(Self { inner, ids })
    }
}

// ── Brute-force i8 cosine baseline (SimSIMD) ────────────────────────────────

pub struct BruteForceI8Index {
    ids: Vec<SignalId>,
    vecs_i8: Vec<Vec<i8>>,
}

fn quantise_i8(v: &[f32]) -> Vec<i8> {
    v.iter()
        .map(|x| (x * 127.0).clamp(-127.0, 127.0) as i8)
        .collect()
}

impl BruteForceI8Index {
    pub fn build(entries: &[SignalEntry]) -> Self {
        let ids = entries.iter().map(|(id, _)| *id).collect();
        let vecs_i8 = entries.iter().map(|(_, v)| quantise_i8(v)).collect();
        Self { ids, vecs_i8 }
    }

    pub fn search(&self, query: &[f32], top_k: usize) -> Vec<SignalHit> {
        let q_i8 = quantise_i8(query);
        let mut scores: Vec<(usize, f32)> = self
            .vecs_i8
            .iter()
            .enumerate()
            .filter_map(|(i, v)| {
                // cos returns distance (1 - similarity); negate for similarity
                let d = i8::cos(&q_i8, v)?;
                Some((i, -(d as f32)))
            })
            .collect();
        scores.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        scores
            .into_iter()
            .take(top_k)
            .map(|(i, s)| (self.ids[i], s))
            .collect()
    }
}

// ── Plain f32 dot-product ground-truth ──────────────────────────────────────

pub fn dot_product_top_k(entries: &[SignalEntry], query: &[f32], top_k: usize) -> Vec<SignalHit> {
    let mut scores: Vec<SignalHit> = entries
        .iter()
        .map(|(id, v)| {
            let s: f32 = v.iter().zip(query).map(|(a, b)| a * b).sum();
            (*id, s)
        })
        .collect();
    scores.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    scores.truncate(top_k);
    scores
}
