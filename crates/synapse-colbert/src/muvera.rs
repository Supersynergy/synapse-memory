//! MUVERA: Multi-Vector Retrieval via Fixed Dimensional Encodings
//! Paper: "MUVERA" (Google 2024)
//!
//! ColBERT token-vecs [seq_len, D] → single FDE vector [n_buckets × D]
//! via locality-sensitive hashing (random hyperplanes), enabling
//! ColBERT-quality recall at dense-ANN speed.
//!
//! Algorithm (paper §3):
//!   1. Project each token-vec [D] → [log2(n_buckets)] via random hyperplanes
//!   2. Assign token to bucket by sign-binarization of projection
//!   3. Per-bucket: sum all assigned token-vecs (zero-pad empty buckets)
//!   4. Concatenate bucket aggregates → FDE [n_buckets × D]

use std::collections::HashMap;

/// Seeded pseudo-random float in [-1, 1] — lcg-based, no deps.
fn lcg_rand(state: &mut u64) -> f32 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    let bits = ((*state >> 33) as u32) | 0x3F800000;
    let f = f32::from_bits(bits) - 1.5;
    f * 2.0 // [-1, 1]
}

/// Build random hyperplane matrix [n_planes × token_dim], seeded.
/// n_planes = log2(n_buckets) rounded up to cover n_buckets.
fn build_hyperplanes(token_dim: usize, n_planes: usize, seed: u64) -> Vec<Vec<f32>> {
    let mut state = seed;
    (0..n_planes)
        .map(|_| {
            let row: Vec<f32> = (0..token_dim).map(|_| lcg_rand(&mut state)).collect();
            // L2-normalize the hyperplane normal
            let norm = row.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
            row.into_iter().map(|x| x / norm).collect()
        })
        .collect()
}

/// Assign a single token-vec to a bucket index (0..n_buckets) via sign-binarization.
fn bucket_for(token: &[f32], hyperplanes: &[Vec<f32>]) -> usize {
    let mut idx = 0usize;
    for (bit, hp) in hyperplanes.iter().enumerate() {
        let proj: f32 = token.iter().zip(hp.iter()).map(|(a, b)| a * b).sum();
        if proj >= 0.0 {
            idx |= 1 << bit;
        }
    }
    idx
}

/// MUVERA Fixed Dimensional Encoding.
///
/// # Arguments
/// * `token_vecs` – ColBERT token representations, each of length `token_dim`
/// * `fde_dim`    – output dimension; must be a multiple of `token_dim`
///                  (`n_buckets = fde_dim / token_dim`)
/// * `seed`       – RNG seed for reproducibility
///
/// # Returns
/// FDE vector of length `fde_dim` = `n_buckets × token_dim`.
pub fn muvera_encode(token_vecs: &[Vec<f32>], fde_dim: usize, seed: u64) -> Vec<f32> {
    if token_vecs.is_empty() {
        return vec![0.0f32; fde_dim];
    }
    let token_dim = token_vecs[0].len();
    assert!(
        token_dim > 0 && fde_dim >= token_dim && fde_dim % token_dim == 0,
        "fde_dim must be a positive multiple of token_dim"
    );

    let n_buckets = fde_dim / token_dim;
    // n_planes = ceil(log2(n_buckets)), min 1
    let n_planes = usize::max(1, (n_buckets as f64).log2().ceil() as usize);

    let hyperplanes = build_hyperplanes(token_dim, n_planes, seed);

    // Aggregate: sum per bucket
    let mut buckets: HashMap<usize, Vec<f32>> = HashMap::new();
    for tv in token_vecs {
        debug_assert_eq!(tv.len(), token_dim);
        let b = bucket_for(tv, &hyperplanes) % n_buckets;
        let acc = buckets.entry(b).or_insert_with(|| vec![0.0f32; token_dim]);
        for (a, x) in acc.iter_mut().zip(tv.iter()) {
            *a += x;
        }
    }

    // Concatenate: bucket 0, 1, ..., n_buckets-1 (zero-pad empty)
    let mut fde = vec![0.0f32; fde_dim];
    for (b, agg) in &buckets {
        let off = b * token_dim;
        fde[off..off + token_dim].copy_from_slice(agg);
    }

    // L2-normalize entire FDE
    let norm = fde.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-9 {
        fde.iter_mut().for_each(|x| *x /= norm);
    }
    fde
}

/// Cosine similarity between two equal-length L2-normalised vectors.
pub fn cosine_sim(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_token_vecs(n: usize, dim: usize, base: f32) -> Vec<Vec<f32>> {
        (0..n)
            .map(|i| {
                let mut v: Vec<f32> = (0..dim)
                    .map(|j| base + (i * dim + j) as f32 * 0.01)
                    .collect();
                let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
                v.iter_mut().for_each(|x| *x /= norm);
                v
            })
            .collect()
    }

    #[test]
    fn deterministic() {
        let vecs = make_token_vecs(8, 16, 1.0);
        let fde1 = muvera_encode(&vecs, 64, 42);
        let fde2 = muvera_encode(&vecs, 64, 42);
        assert_eq!(fde1, fde2, "same input+seed must give same FDE");
    }

    #[test]
    fn similar_docs_high_cosine() {
        // Two very similar token sets (small perturbation)
        let vecs_a = make_token_vecs(8, 16, 1.0);
        let vecs_b = make_token_vecs(8, 16, 1.001); // tiny shift
        let fde_a = muvera_encode(&vecs_a, 64, 42);
        let fde_b = muvera_encode(&vecs_b, 64, 42);
        let sim = cosine_sim(&fde_a, &fde_b);
        assert!(
            sim > 0.5,
            "similar docs should have cosine > 0.5, got {sim:.4}"
        );
    }

    #[test]
    fn dissimilar_docs_low_cosine() {
        let dim = 16;
        let vecs_b: Vec<Vec<f32>> = (0..8)
            .map(|_| {
                let v: Vec<f32> = (0..dim)
                    .map(|j| if j % 2 == 0 { 1.0 } else { -1.0 })
                    .collect();
                let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                v.into_iter().map(|x| x / norm).collect()
            })
            .collect();
        let va: Vec<Vec<f32>> = (0..8)
            .map(|i| {
                let mut v = vec![0.0f32; dim];
                v[i % dim] = 1.0;
                v
            })
            .collect();

        let fde_a = muvera_encode(&va, 64, 42);
        let fde_b = muvera_encode(&vecs_b, 64, 42);
        let sim = cosine_sim(&fde_a, &fde_b);
        assert!(
            sim < 0.7,
            "dissimilar docs should have lower cosine, got {sim:.4}"
        );
    }

    #[test]
    fn empty_input() {
        let fde = muvera_encode(&[], 128, 0);
        assert_eq!(fde.len(), 128);
        assert!(fde.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn different_fde_dims() {
        let vecs = make_token_vecs(4, 16, 0.5);
        for &fde_dim in &[16, 32, 64, 128, 256] {
            let fde = muvera_encode(&vecs, fde_dim, 7);
            assert_eq!(fde.len(), fde_dim);
        }
    }

    #[test]
    fn l2_normalized_output() {
        let vecs = make_token_vecs(6, 16, 0.3);
        let fde = muvera_encode(&vecs, 64, 13);
        let norm: f32 = fde.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-5,
            "FDE should be L2-normalised, norm={norm}"
        );
    }
}
