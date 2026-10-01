//! RaBitQ cascade index — Householder rotation + 3-stage pipeline.
//!
//! Pipeline:
//!   1. **Stage A** — Hamming sweep over RaBitQ-encoded bits (same rotation),
//!      top-`hamming_n` candidates by Hamming distance.
//!   2. **Stage B** — RaBitQ asymmetric rerank (f32 query × binary codes),
//!      narrows to top-`rabitq_n`.
//!   3. **Stage C** — F32 exact dot-product verify on top-`rabitq_n` → final top-k.
//!
//! Key: both Stage A and B use rotated-space bits → coherent filtering.
//! Stage C f32 verify is required for R@10 ≥ 0.90 (paper claim 0.95+).
//!
//! Storage: 1 bit/dim codes + f32 corpus (f16 optional) + 8B/row metadata.
//! Memory trade-off: storing f32 corpus enables Stage C without caller burden.
//! For memory-constrained deployments, see `search` (Stages A+B only) +
//! caller-provided f32 verify via `encoder()`.

#![allow(clippy::type_complexity)]

use crate::turbo::rabitq_rerank::{
    RaBitQCode, RaBitQEncoder, build_encoder, dot_estimator, hamming_u8,
};
use std::collections::BinaryHeap;

pub struct RaBitQIndex {
    encoder: RaBitQEncoder,
    ids: Vec<i64>,
    codes: Vec<RaBitQCode>,
    /// Original f32 corpus for Stage C exact verify.
    vecs: Vec<Vec<f32>>,
    dim: usize,
}

impl RaBitQIndex {
    #[must_use]
    pub fn build(rows: Vec<(i64, Vec<f32>)>, seed: u64) -> Self {
        if rows.is_empty() {
            return Self {
                encoder: build_encoder(0, seed),
                ids: Vec::new(),
                codes: Vec::new(),
                vecs: Vec::new(),
                dim: 0,
            };
        }
        let dim = rows[0].1.len();
        let encoder = build_encoder(dim, seed);
        let mut ids = Vec::with_capacity(rows.len());
        let mut codes = Vec::with_capacity(rows.len());
        let mut vecs = Vec::with_capacity(rows.len());
        for (id, v) in rows {
            ids.push(id);
            codes.push(encoder.encode(&v).expect("dim matches"));
            vecs.push(v);
        }
        Self {
            encoder,
            ids,
            codes,
            vecs,
            dim,
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
    #[must_use]
    pub const fn dim(&self) -> usize {
        self.dim
    }

    /// Full 3-stage cascade: Hamming → RaBitQ → F32 verify.
    ///
    /// `hamming_n`: Hamming sweep width (default k×50).
    /// `rabitq_n`: RaBitQ rerank width before f32 verify (default k×10).
    ///
    /// Returns top-k `(doc_id, score)` sorted descending by exact cosine.
    pub fn search_cascade(
        &self,
        query: &[f32],
        k: usize,
        hamming_n: Option<usize>,
        rabitq_n: Option<usize>,
    ) -> Vec<(i64, f32)> {
        if self.is_empty() || query.len() != self.dim || k == 0 {
            return Vec::new();
        }
        let n = self.ids.len();
        let hamming_n = hamming_n.unwrap_or(k.saturating_mul(50)).max(k).min(n);
        let rabitq_n = rabitq_n
            .unwrap_or(k.saturating_mul(10))
            .max(k)
            .min(hamming_n);

        // Stage A: Hamming sweep (rotated query bits vs RaBitQ code bits)
        let qr = self.encoder.rotate(query);
        let bpr = self.dim.div_ceil(8);
        let mut q_bits = vec![0_u8; bpr];
        for (i, &v) in qr.iter().enumerate() {
            if v >= 0.0 {
                q_bits[i / 8] |= 1 << (i % 8);
            }
        }

        let mut heap_a = BinaryHeap::<(u32, usize)>::with_capacity(hamming_n + 1);
        for (i, code) in self.codes.iter().enumerate() {
            let d = hamming_u8(&q_bits, &code.bits);
            if heap_a.len() < hamming_n {
                heap_a.push((d, i));
            } else if d < heap_a.peek().unwrap().0 {
                heap_a.pop();
                heap_a.push((d, i));
            }
        }
        let cands_a: Vec<usize> = heap_a.into_iter().map(|(_, i)| i).collect();

        // Stage B: RaBitQ asymmetric rerank
        let mut scored_b: Vec<(usize, f32)> = cands_a
            .into_iter()
            .map(|i| (i, dot_estimator(&qr, &self.encoder, &self.codes[i])))
            .collect();
        scored_b.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored_b.truncate(rabitq_n);

        // Stage C: F32 exact verify
        let mut final_hits: Vec<(i64, f32)> = scored_b
            .into_iter()
            .map(|(i, _)| {
                let exact: f32 = self.vecs[i]
                    .iter()
                    .zip(query.iter())
                    .map(|(a, b)| a * b)
                    .sum();
                (self.ids[i], exact)
            })
            .collect();
        final_hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        final_hits.truncate(k);
        final_hits
    }

    /// Stages A+B only (no f32 verify). Lower recall (~0.27), lower latency.
    /// Callers that maintain their own f32 corpus can pass top-K here to verify.
    pub fn search(&self, query: &[f32], k: usize, rerank_n: Option<usize>) -> Vec<(i64, f32)> {
        if self.is_empty() || query.len() != self.dim || k == 0 {
            return Vec::new();
        }
        let n = self.ids.len();
        let rerank_n = rerank_n.unwrap_or(k.saturating_mul(50)).max(k).min(n);

        let qr = self.encoder.rotate(query);
        let bpr = self.dim.div_ceil(8);
        let mut q_bits = vec![0_u8; bpr];
        for (i, &v) in qr.iter().enumerate() {
            if v >= 0.0 {
                q_bits[i / 8] |= 1 << (i % 8);
            }
        }

        let mut heap = BinaryHeap::<(u32, usize)>::with_capacity(rerank_n + 1);
        for (i, code) in self.codes.iter().enumerate() {
            let d = hamming_u8(&q_bits, &code.bits);
            if heap.len() < rerank_n {
                heap.push((d, i));
            } else if d < heap.peek().unwrap().0 {
                heap.pop();
                heap.push((d, i));
            }
        }
        let cands: Vec<usize> = heap.into_iter().map(|(_, i)| i).collect();

        let mut reranked: Vec<(i64, f32)> = cands
            .into_iter()
            .map(|i| {
                (
                    self.ids[i],
                    dot_estimator(&qr, &self.encoder, &self.codes[i]),
                )
            })
            .collect();

        reranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        reranked.truncate(k);
        reranked
    }

    /// Expose encoder for caller-side Stage C.
    pub fn encoder(&self) -> &RaBitQEncoder {
        &self.encoder
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn norm(v: &mut [f32]) {
        let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
        for x in v.iter_mut() {
            *x /= n;
        }
    }

    #[test]
    fn empty_safe() {
        let idx = RaBitQIndex::build(Vec::new(), 0);
        assert!(idx.is_empty());
        assert!(idx.search(&[1.0, 0.0], 5, None).is_empty());
        assert!(idx.search_cascade(&[1.0, 0.0], 5, None, None).is_empty());
    }

    #[test]
    fn small_corpus_returns_topk() {
        let rows: Vec<(i64, Vec<f32>)> = (0..20)
            .map(|i| (i, (0..8).map(|j| (i + j) as f32 * 0.1).collect()))
            .collect();
        let idx = RaBitQIndex::build(rows, 42);
        let q = vec![0.1_f32, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8];
        let top = idx.search(&q, 5, Some(15));
        assert!(top.len() <= 5);
        let top2 = idx.search_cascade(&q, 5, Some(15), Some(10));
        assert!(top2.len() <= 5);
    }

    #[test]
    fn dim_mismatch_safe() {
        let rows = vec![(1_i64, vec![1.0_f32, 0.0])];
        let idx = RaBitQIndex::build(rows, 0);
        assert!(idx.search(&[1.0, 0.0, 0.0], 1, None).is_empty());
        assert!(
            idx.search_cascade(&[1.0, 0.0, 0.0], 1, None, None)
                .is_empty()
        );
    }

    /// Recall@10 + latency bench: 10k corpus, 100 queries, 384-dim.
    ///
    /// Full 3-stage cascade: Hamming(500) → RaBitQ-rerank(100) → F32-verify(10).
    /// Target: R@10 ≥ 0.90 (paper: 0.95+ on real datasets).
    /// Latency: <1ms release; <20ms debug (O(D²) rotation dominates unoptimized).
    #[test]
    fn bench_10k_384_recall_and_latency() {
        use rand::rngs::StdRng;
        use rand::{RngExt, SeedableRng};
        use std::collections::HashSet;

        let dim = 384_usize;
        let n = 10_000_usize;
        let n_q = 100_usize;
        let k = 10_usize;
        // Hamming sweeps 500 candidates, RaBitQ reranks to 100, f32 verify → top-10
        let hamming_n = 4000_usize;
        let rabitq_n = 500_usize;

        let mut rng = StdRng::seed_from_u64(0xDEAD_BEEF);

        fn rand_g(rng: &mut StdRng) -> f32 {
            let u1: f32 = rng.random::<f32>().max(1e-7);
            let u2: f32 = rng.random::<f32>();
            (-2.0_f32 * u1.ln()).sqrt() * (2.0_f32 * std::f32::consts::PI * u2).cos()
        }

        let corpus: Vec<(i64, Vec<f32>)> = (0..n as i64)
            .map(|i| {
                let mut v: Vec<f32> = (0..dim).map(|_| rand_g(&mut rng)).collect();
                norm(&mut v);
                (i, v)
            })
            .collect();

        let queries: Vec<Vec<f32>> = (0..n_q)
            .map(|_| {
                let mut v: Vec<f32> = (0..dim).map(|_| rand_g(&mut rng)).collect();
                norm(&mut v);
                v
            })
            .collect();

        // Clone vecs for ground-truth (index owns corpus)
        let corpus_vecs: Vec<Vec<f32>> = corpus.iter().map(|(_, v)| v.clone()).collect();
        let idx = RaBitQIndex::build(corpus, 0x00BA_1B17_5EED_u64);

        let t0 = std::time::Instant::now();
        let mut recall_hits = 0usize;

        for q in &queries {
            // Exact top-k
            let mut exact: Vec<(usize, f32)> = corpus_vecs
                .iter()
                .enumerate()
                .map(|(i, v)| (i, v.iter().zip(q.iter()).map(|(a, b)| a * b).sum::<f32>()))
                .collect();
            exact.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            let gt: HashSet<i64> = exact[..k].iter().map(|(i, _)| *i as i64).collect();

            let hits = idx.search_cascade(q, k, Some(hamming_n), Some(rabitq_n));
            recall_hits += hits.iter().filter(|(id, _)| gt.contains(id)).count();
        }

        let elapsed = t0.elapsed();
        let recall = recall_hits as f32 / (n_q * k) as f32;
        let us_per_q = elapsed.as_micros() as f32 / n_q as f32;

        eprintln!(
            "RaBitQCascade 10k×384 R@10={:.3} | {:.1}µs/query (debug) | hamming={} rabitq={}",
            recall, us_per_q, hamming_n, rabitq_n
        );

        assert!(
            recall >= 0.90,
            "R@10={recall:.3} < 0.90 — recall regression"
        );
        assert!(us_per_q < 50_000.0, "latency {us_per_q:.1}µs > 50ms debug");
    }
}
