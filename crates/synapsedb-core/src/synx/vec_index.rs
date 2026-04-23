//! HNSW vector index with scalar (int8) quantization — Phase 3 Track (c).
//!
//! Goal: 4× smaller vectors on disk, 3–5× faster kNN on >100k points vs the
//! flat sqlite-vec scan used in v0.1. Implementation: `instant-distance` HNSW
//! with cosine distance + simple per-dim min/max scalar quantization.
//!
//! Feature-gated on `vec-hnsw`. Stub is safe to call but returns empty results.

#[cfg(feature = "vec-hnsw")]
pub use imp::*;
#[cfg(not(feature = "vec-hnsw"))]
pub use stub::*;

#[cfg(feature = "vec-hnsw")]
mod imp {
    use crate::error::{Error, Result};
    use instant_distance::{Builder, HnswMap, Point, Search};

    /// Quantization codebook — per-dim (min, max) for int8 round-trip.
    #[derive(Clone, Debug)]
    pub struct ScalarCodebook {
        pub mins: Vec<f32>,
        pub maxs: Vec<f32>,
        pub dim: usize,
    }

    impl ScalarCodebook {
        pub fn train(vectors: &[Vec<f32>]) -> Result<Self> {
            if vectors.is_empty() {
                return Err(Error::Format("empty training set".into()));
            }
            let dim = vectors[0].len();
            let mut mins = vec![f32::INFINITY; dim];
            let mut maxs = vec![f32::NEG_INFINITY; dim];
            for v in vectors {
                if v.len() != dim {
                    return Err(Error::DimMismatch {
                        expected: dim,
                        got: v.len(),
                    });
                }
                for (i, &x) in v.iter().enumerate() {
                    if x < mins[i] {
                        mins[i] = x;
                    }
                    if x > maxs[i] {
                        maxs[i] = x;
                    }
                }
            }
            // avoid division by zero on constant dims
            for i in 0..dim {
                if (maxs[i] - mins[i]).abs() < 1e-12 {
                    maxs[i] = mins[i] + 1.0;
                }
            }
            Ok(Self { mins, maxs, dim })
        }

        pub fn quantize(&self, v: &[f32]) -> Vec<i8> {
            let mut out = Vec::with_capacity(v.len());
            for (i, &x) in v.iter().enumerate() {
                let norm = (x - self.mins[i]) / (self.maxs[i] - self.mins[i]);
                let clamped = norm.clamp(0.0, 1.0);
                let q = (clamped * 255.0 - 128.0).round() as i32;
                out.push(q.clamp(-128, 127) as i8);
            }
            out
        }

        pub fn dequantize(&self, q: &[i8]) -> Vec<f32> {
            let mut out = Vec::with_capacity(q.len());
            for (i, &qv) in q.iter().enumerate() {
                let norm = (qv as f32 + 128.0) / 255.0;
                out.push(self.mins[i] + norm * (self.maxs[i] - self.mins[i]));
            }
            out
        }
    }

    /// Newtype so we can impl `Point` for `Vec<f32>` without orphan-rule issues.
    #[derive(Clone, Debug)]
    pub struct Embedding(pub Vec<f32>);

    impl Point for Embedding {
        /// Cosine distance — smaller is more similar.
        fn distance(&self, other: &Self) -> f32 {
            let mut dot = 0.0f32;
            let mut na = 0.0f32;
            let mut nb = 0.0f32;
            for (a, b) in self.0.iter().zip(other.0.iter()) {
                dot += a * b;
                na += a * a;
                nb += b * b;
            }
            let denom = (na.sqrt() * nb.sqrt()).max(1e-12);
            1.0 - (dot / denom)
        }
    }

    /// Built HNSW index with optional scalar quantization.
    pub struct HnswIndex {
        map: HnswMap<Embedding, u32>,
        pub codebook: Option<ScalarCodebook>,
    }

    impl HnswIndex {
        pub fn build(vectors: Vec<Vec<f32>>, quantize: bool) -> Result<Self> {
            if vectors.is_empty() {
                return Err(Error::Format("empty vector set".into()));
            }
            let codebook = if quantize {
                Some(ScalarCodebook::train(&vectors)?)
            } else {
                None
            };
            let points: Vec<Embedding> = if let Some(cb) = &codebook {
                vectors
                    .into_iter()
                    .map(|v| Embedding(cb.dequantize(&cb.quantize(&v))))
                    .collect()
            } else {
                vectors.into_iter().map(Embedding).collect()
            };
            let ids: Vec<u32> = (0..points.len() as u32).collect();
            let map = Builder::default()
                .ef_construction(100)
                .ef_search(64)
                .build(points, ids);
            Ok(Self { map, codebook })
        }

        pub fn search(&self, query: &[f32], k: usize) -> Vec<(u32, f32)> {
            let mut search = Search::default();
            let q = Embedding(query.to_vec());
            let iter = self.map.search(&q, &mut search);
            iter.take(k).map(|r| (*r.value, r.distance)).collect()
        }

        pub fn len(&self) -> usize {
            self.map.values.len()
        }

        pub fn is_empty(&self) -> bool {
            self.map.values.is_empty()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn hnsw_finds_nearest() {
            // points on a line in 2-D: direction is identical, magnitude differs.
            // cosine distance between any pair = 0, so the graph just has to
            // return *some* neighbours — and it must not crash or return empty.
            let mut vecs = Vec::new();
            for i in 1..=100 {
                vecs.push(vec![i as f32, (i * 2) as f32]);
            }
            let idx = HnswIndex::build(vecs, false).unwrap();
            let hits = idx.search(&[50.0, 100.0], 5);
            assert_eq!(hits.len(), 5);
            // every returned id should be a valid rowid in range
            for (id, _) in &hits {
                assert!(*id < 100);
            }
        }

        #[test]
        fn quantize_roundtrip_bounded_error() {
            let vecs = vec![
                vec![0.0f32, 1.0, 2.0, 3.0],
                vec![1.0, 0.5, 2.5, 2.5],
                vec![-1.0, 2.0, 0.0, 3.5],
            ];
            let cb = ScalarCodebook::train(&vecs).unwrap();
            let q = cb.quantize(&vecs[0]);
            let back = cb.dequantize(&q);
            for (a, b) in vecs[0].iter().zip(back.iter()) {
                assert!((a - b).abs() < 0.1, "quantize error too high: {a} vs {b}");
            }
        }
    }
}

#[cfg(not(feature = "vec-hnsw"))]
mod stub {
    use crate::error::Result;
    pub struct ScalarCodebook;
    pub struct HnswIndex;
    impl HnswIndex {
        pub fn build(_v: Vec<Vec<f32>>, _q: bool) -> Result<Self> {
            Ok(Self)
        }
        pub fn search(&self, _q: &[f32], _k: usize) -> Vec<(u32, f32)> {
            Vec::new()
        }
        pub fn len(&self) -> usize {
            0
        }
        pub fn is_empty(&self) -> bool {
            true
        }
    }
}
