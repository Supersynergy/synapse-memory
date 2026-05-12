//! RRF — implementation lives in synapse-engine (closed-source crate).

pub fn rrf_fuse_simd(ranks_a: &[f64], ranks_b: &[f64], k: f64) -> (Vec<f64>, Vec<f64>) {
    let scores_a = reciprocal_ranks(ranks_a, k);
    let scores_b = reciprocal_ranks(ranks_b, k);
    (scores_a, scores_b)
}

#[inline]
pub fn reciprocal_ranks(ranks: &[f64], k: f64) -> Vec<f64> {
    // rrf_fuse(ranks, &[], k) yields 1/(k+r) for each element (b side empty)
    synapse_engine::rrf::rrf_fuse(ranks, &[], k)
}

#[inline]
pub fn distance_to_score(distances: &[f32]) -> Vec<f32> {
    synapse_engine::rrf::distance_to_score(distances)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reciprocal_ranks_basic() {
        let ranks = vec![1.0, 2.0, 3.0];
        let k = 60.0;
        let out = reciprocal_ranks(&ranks, k);
        assert_eq!(out.len(), 3);
        assert!((out[0] - 1.0 / 61.0).abs() < 1e-12);
        assert!((out[1] - 1.0 / 62.0).abs() < 1e-12);
        assert!((out[2] - 1.0 / 63.0).abs() < 1e-12);
    }

    #[test]
    fn test_rrf_fuse_simd_lengths() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0];
        let (sa, sb) = rrf_fuse_simd(&a, &b, 60.0);
        assert_eq!(sa.len(), 3);
        assert_eq!(sb.len(), 2);
    }

    #[test]
    fn distance_to_score_basic() {
        let d = vec![0.0_f32, 1.0, 3.0, 7.0];
        let s = distance_to_score(&d);
        assert!((s[0] - 1.0).abs() < 1e-6);
        assert!((s[1] - 0.5).abs() < 1e-6);
        assert!((s[2] - 0.25).abs() < 1e-6);
        assert!((s[3] - 0.125).abs() < 1e-6);
    }

    #[test]
    fn test_rrf_scores_sum_matches_scalar() {
        let ranks: Vec<f64> = (1..=10).map(|i| i as f64).collect();
        let k = 60.0;
        let out = reciprocal_ranks(&ranks, k);
        let expected: Vec<f64> = ranks.iter().map(|&r| 1.0 / (k + r)).collect();
        for (a, b) in out.iter().zip(expected.iter()) {
            assert!((a - b).abs() < 1e-15);
        }
    }
}
