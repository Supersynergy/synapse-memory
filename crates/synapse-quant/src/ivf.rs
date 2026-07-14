//! IVF (Inverted File) coarse quantizer — partitions vectors into clusters.
//!
//! Pattern: assign each vec to nearest of `n_clusters` centroids.
//! Search: probe top-N nearest centroids, scan only those clusters.
//! Reduces 1M → ~10k scanned per query (100× speedup) at cost of recall.
//!
//! Mining-first: simpler than FAISS IVF (no PQ inside cells).
//! Use cosine on L2-normalized inputs. Lloyd's kmeans with k-means++ init.

use crate::QuantError;

pub struct Ivf {
    dim: usize,
    pub centroids: Vec<Vec<f32>>, // n_clusters × dim, L2-normalized
}

impl Ivf {
    pub fn new(dim: usize, n_clusters: usize) -> Self {
        Self {
            dim,
            centroids: Vec::with_capacity(n_clusters),
        }
    }

    /// k-means++ init: spread centroids based on D² weighted distribution.
    fn kpp_init(&mut self, corpus: &[&[f32]], n_clusters: usize, seed: u64) {
        if corpus.is_empty() {
            return;
        }
        let mut rng_state = seed;
        let next = |s: &mut u64| -> usize {
            *s ^= *s << 13;
            *s ^= *s >> 7;
            *s ^= *s << 17;
            (*s as usize) % corpus.len()
        };
        // Pick first centroid randomly
        self.centroids.clear();
        self.centroids.push(corpus[next(&mut rng_state)].to_vec());
        // Pick subsequent: prob ∝ D² to nearest existing centroid
        while self.centroids.len() < n_clusters {
            let dists: Vec<f32> = corpus
                .iter()
                .map(|v| {
                    self.centroids
                        .iter()
                        .map(|c| {
                            let d: f32 = v.iter().zip(c.iter()).map(|(a, b)| (a - b).powi(2)).sum();
                            d
                        })
                        .fold(f32::INFINITY, f32::min)
                })
                .collect();
            let total: f32 = dists.iter().sum();
            if total < 1e-10 {
                break;
            }
            // Weighted random sample (linear scan, simple)
            rng_state ^= rng_state << 13;
            rng_state ^= rng_state >> 7;
            rng_state ^= rng_state << 17;
            let r = (rng_state as f32 / u64::MAX as f32).abs() * total;
            let mut acc = 0.0_f32;
            let mut idx = 0;
            for (i, d) in dists.iter().enumerate() {
                acc += d;
                if acc >= r {
                    idx = i;
                    break;
                }
            }
            self.centroids.push(corpus[idx].to_vec());
        }
    }

    /// Train centroids via Lloyd's kmeans. iters=10 typical.
    pub fn train(
        &mut self,
        corpus: &[&[f32]],
        n_clusters: usize,
        iters: usize,
    ) -> Result<(), QuantError> {
        if corpus.is_empty() {
            return Ok(());
        }
        let dim = corpus[0].len();
        if dim != self.dim {
            return Err(QuantError::DimMismatch {
                expected: self.dim,
                actual: dim,
            });
        }
        self.kpp_init(corpus, n_clusters, 42);
        for _ in 0..iters {
            let mut sums = vec![vec![0.0_f32; dim]; self.centroids.len()];
            let mut counts = vec![0_usize; self.centroids.len()];
            for v in corpus {
                let c = self.assign(v);
                for (s, x) in sums[c].iter_mut().zip(v.iter()) {
                    *s += *x;
                }
                counts[c] += 1;
            }
            for (i, sum) in sums.into_iter().enumerate() {
                if counts[i] == 0 {
                    continue;
                }
                let avg: Vec<f32> = sum.iter().map(|x| x / counts[i] as f32).collect();
                // L2-normalize new centroid for cosine
                let n: f32 = avg.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-10);
                self.centroids[i] = avg.iter().map(|x| x / n).collect();
            }
        }
        Ok(())
    }

    /// Assign vec to nearest centroid (cosine — assumes both L2-normalized).
    pub fn assign(&self, v: &[f32]) -> usize {
        let mut best_idx = 0;
        let mut best_dot = f32::MIN;
        for (i, c) in self.centroids.iter().enumerate() {
            let d: f32 = v.iter().zip(c.iter()).map(|(a, b)| a * b).sum();
            if d > best_dot {
                best_dot = d;
                best_idx = i;
            }
        }
        best_idx
    }

    /// Top-k nearest centroids — used for probe selection at query.
    pub fn topk_centroids(&self, query: &[f32], k: usize) -> Vec<usize> {
        let mut scored: Vec<(usize, f32)> = self
            .centroids
            .iter()
            .enumerate()
            .map(|(i, c)| (i, query.iter().zip(c.iter()).map(|(a, b)| a * b).sum()))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().take(k).map(|(i, _)| i).collect()
    }

    pub fn n_clusters(&self) -> usize {
        self.centroids.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn norm(mut v: Vec<f32>) -> Vec<f32> {
        let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-10);
        for x in v.iter_mut() {
            *x /= n;
        }
        v
    }

    #[test]
    fn ivf_train_assign() {
        let raw = vec![
            norm(vec![1.0, 0.0]),
            norm(vec![0.95, 0.05]),
            norm(vec![0.9, 0.1]),
            norm(vec![0.0, 1.0]),
            norm(vec![0.05, 0.95]),
            norm(vec![0.1, 0.9]),
        ];
        let corpus: Vec<&[f32]> = raw.iter().map(|v| v.as_slice()).collect();
        let mut ivf = Ivf::new(2, 2);
        ivf.train(&corpus, 2, 5).unwrap();
        // 2 clusters: ~(1,0) region + ~(0,1) region
        assert_eq!(ivf.centroids.len(), 2);
        let a = ivf.assign(&[1.0, 0.0]);
        let b = ivf.assign(&[0.0, 1.0]);
        assert!(a != b, "different regions should hit different clusters");
    }

    #[test]
    fn topk_centroids_works() {
        let mut ivf = Ivf::new(2, 3);
        ivf.centroids = vec![vec![1.0, 0.0], vec![0.0, 1.0], vec![0.7071, 0.7071]];
        let top = ivf.topk_centroids(&[1.0, 0.0], 2);
        assert_eq!(top[0], 0); // exact match
    }
}
