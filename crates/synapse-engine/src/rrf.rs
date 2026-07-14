/// RRF (Reciprocal Rank Fusion) — IP kernel, migrated from synapse-core Phase 10 Day 5.
pub fn rrf_fuse(ranks_a: &[f64], ranks_b: &[f64], k: f64) -> Vec<f64> {
    let len = ranks_a.len().max(ranks_b.len());
    let mut out = vec![0.0_f64; len];
    for (i, v) in out.iter_mut().enumerate() {
        if i < ranks_a.len() {
            *v += 1.0 / (k + ranks_a[i]);
        }
        if i < ranks_b.len() {
            *v += 1.0 / (k + ranks_b[i]);
        }
    }
    out
}

#[inline]
pub fn distance_to_score(distances: &[f32]) -> Vec<f32> {
    let mut out = vec![0.0_f32; distances.len()];
    distance_to_score_inplace(distances, &mut out);
    out
}

/// Zero-alloc variant: write scores into pre-allocated `out` slice.
/// NEON path: vrecpeq_f32 + 2×NR → ~23-bit accurate reciprocal of (1+d).
/// Caller must ensure `out.len() >= distances.len()`.
#[inline]
pub fn distance_to_score_inplace(distances: &[f32], out: &mut [f32]) {
    debug_assert!(out.len() >= distances.len());
    let n = distances.len();

    #[cfg(target_arch = "aarch64")]
    {
        use std::arch::aarch64::*;
        let mut i = 0usize;
        let one = unsafe { vdupq_n_f32(1.0) };
        while i + 4 <= n {
            unsafe {
                let d = vld1q_f32(distances.as_ptr().add(i));
                let denom = vaddq_f32(one, d); // 1 + d
                let est = vrecpeq_f32(denom);
                let est = vmulq_f32(est, vrecpsq_f32(denom, est));
                let est = vmulq_f32(est, vrecpsq_f32(denom, est));
                vst1q_f32(out.as_mut_ptr().add(i), est);
            }
            i += 4;
        }
        while i < n {
            out[i] = 1.0 / (1.0 + distances[i]);
            i += 1;
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        for (o, d) in out[..n].iter_mut().zip(distances.iter()) {
            *o = 1.0_f32 / (1.0_f32 + d);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrf_fuse_same_lengths() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0, 3.0];
        let out = rrf_fuse(&a, &b, 60.0);
        assert_eq!(out.len(), 3);
        let expected = 2.0 / (60.0 + 1.0);
        assert!((out[0] - expected).abs() < 1e-12);
    }

    #[test]
    fn rrf_fuse_unequal_lengths() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0];
        let out = rrf_fuse(&a, &b, 60.0);
        assert_eq!(out.len(), 3);
        // index 2: only a contributes
        assert!((out[2] - 1.0 / 63.0).abs() < 1e-12);
    }

    #[test]
    fn distance_to_score_values() {
        let d = vec![0.0_f32, 1.0, 3.0, 7.0];
        let s = distance_to_score(&d);
        assert!((s[0] - 1.0).abs() < 1e-6);
        assert!((s[1] - 0.5).abs() < 1e-6);
        assert!((s[2] - 0.25).abs() < 1e-6);
        assert!((s[3] - 0.125).abs() < 1e-6);
    }
}
