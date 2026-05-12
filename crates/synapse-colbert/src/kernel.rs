//! ColBERT MaxSim late-interaction kernel.
//! max_sim(Q, D) = Σ_{qi ∈ Q} max_{dj ∈ D} cosine(qi, dj)
//! With L2-normalised vectors: cosine = dot product.

/// ColBERT max-sim score.
/// Both slices must contain L2-normalised f32 vectors of equal dim.
/// O(|Q| × |D| × dim) — acceptable for reranking ~100 candidates.
pub fn max_sim(query_vecs: &[Vec<f32>], doc_vecs: &[Vec<f32>]) -> f32 {
    if query_vecs.is_empty() || doc_vecs.is_empty() {
        return 0.0;
    }
    query_vecs
        .iter()
        .map(|qv| {
            doc_vecs
                .iter()
                .map(|dv| dot(qv, dv))
                .fold(f32::NEG_INFINITY, f32::max)
        })
        .sum()
}

#[inline(always)]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    // unroll-4 for compiler auto-vec; same pattern as synapse-kernel f32_l2
    let n = a.len();
    let mut acc0 = 0f32;
    let mut acc1 = 0f32;
    let mut acc2 = 0f32;
    let mut acc3 = 0f32;
    let mut i = 0;
    while i + 4 <= n {
        acc0 += unsafe { *a.get_unchecked(i) * *b.get_unchecked(i) };
        acc1 += unsafe { *a.get_unchecked(i + 1) * *b.get_unchecked(i + 1) };
        acc2 += unsafe { *a.get_unchecked(i + 2) * *b.get_unchecked(i + 2) };
        acc3 += unsafe { *a.get_unchecked(i + 3) * *b.get_unchecked(i + 3) };
        i += 4;
    }
    while i < n {
        acc0 += unsafe { *a.get_unchecked(i) * *b.get_unchecked(i) };
        i += 1;
    }
    (acc0 + acc1) + (acc2 + acc3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(dim: usize, idx: usize) -> Vec<f32> {
        let mut v = vec![0f32; dim];
        v[idx] = 1.0;
        v
    }

    #[test]
    fn identical_vecs() {
        let q = vec![unit(4, 0)];
        let d = vec![unit(4, 0)];
        assert!((max_sim(&q, &d) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn orthogonal_zero() {
        let q = vec![unit(4, 0)];
        let d = vec![unit(4, 1)];
        assert!(max_sim(&q, &d).abs() < 1e-6);
    }

    #[test]
    fn multi_token_sum() {
        // 2 query tokens, each perfectly matches one doc token → score = 2.0
        let q = vec![unit(4, 0), unit(4, 1)];
        let d = vec![unit(4, 0), unit(4, 1), unit(4, 2)];
        assert!((max_sim(&q, &d) - 2.0).abs() < 1e-6);
    }
}
