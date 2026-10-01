//! Matryoshka Representation Learning (MRL) — embedding truncation.
//!
//! For MRL-trained models (BGE-M3, bge-base-en-v1.5 w/ MRL, e5-mrl, OpenAI-3),
//! the first *k* dimensions of the full vector are themselves a valid
//! lower-dimensional embedding. Truncating + L2-renormalizing gives a 3–6×
//! speed-up on matvec with near-identical recall.
//!
//! | Full dim | MRL k | recall@10 retention | matvec speed-up |
//! |:--------:|:-----:|:-------------------:|:---------------:|
//! | 1024     | 256   | ~99 %               | 4×              |
//! | 768      | 128   | ~97 %               | 6×              |
//! | 384      | 128   | ~95 %               | 3×              |
//!
//! Use [`truncate_row`] on the hot path; [`truncate_rows`] for a full corpus
//! re-compression pass.

/// Truncate and L2-renormalize a single embedding to the first `k` dims.
///
/// Returns an empty vec if `k > src.len()` or `k == 0`.
#[must_use]
pub fn truncate_row(src: &[f32], k: usize) -> Vec<f32> {
    if k == 0 || k > src.len() {
        return Vec::new();
    }
    let mut out: Vec<f32> = src[..k].to_vec();
    let norm_sq: f32 = out.iter().map(|x| x * x).sum();
    let norm = norm_sq.sqrt().max(1e-12);
    let inv = 1.0 / norm;
    for x in &mut out {
        *x *= inv;
    }
    out
}

/// Binary-Matryoshka: truncate to first `k` dims + sign-binarize + pack.
///
/// Output bytes per row = `ceil(k / 8)`. At k=128 that's 16 bytes — 24× smaller
/// than 384-d fp32. Trades ~10 points recall for storage; recoverable via
/// int8-rescore downstream.
#[must_use]
pub fn truncate_to_binary(src: &[f32], k: usize) -> Vec<u8> {
    if k == 0 || k > src.len() {
        return Vec::new();
    }
    let bpr = k.div_ceil(8);
    let mut out = vec![0_u8; bpr];
    for (i, &v) in src.iter().take(k).enumerate() {
        if v > 0.0 {
            out[i / 8] |= 1 << (i % 8);
        }
    }
    out
}

/// Bulk variant: re-compress a whole corpus. Allocates one contiguous vec of
/// length `n * k` to stay cache-friendly for downstream matvec.
#[must_use]
pub fn truncate_rows(src: &[Vec<f32>], k: usize) -> Vec<f32> {
    if src.is_empty() || k == 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(src.len() * k);
    for row in src {
        let r = truncate_row(row, k);
        out.extend_from_slice(&r);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_produces_unit_vector() {
        let v = vec![3.0_f32, 4.0, 0.0, 0.0];
        let t = truncate_row(&v, 2);
        let n: f32 = t.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((n - 1.0).abs() < 1e-5);
        assert!((t[0] - 0.6).abs() < 1e-5);
        assert!((t[1] - 0.8).abs() < 1e-5);
    }

    #[test]
    fn truncate_out_of_range_returns_empty() {
        assert!(truncate_row(&[1.0, 2.0], 0).is_empty());
        assert!(truncate_row(&[1.0, 2.0], 5).is_empty());
    }

    #[test]
    fn binary_matryoshka_packs_bits() {
        let v = vec![1.0_f32, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 9.0];
        let out = truncate_to_binary(&v, 8);
        assert_eq!(out, vec![0b0101_0101]);
    }

    #[test]
    fn binary_matryoshka_respects_k() {
        let v: Vec<f32> = (0..16)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        assert_eq!(truncate_to_binary(&v, 8).len(), 1);
        assert_eq!(truncate_to_binary(&v, 16).len(), 2);
    }

    #[test]
    fn binary_matryoshka_out_of_range_empty() {
        assert!(truncate_to_binary(&[], 0).is_empty());
        assert!(truncate_to_binary(&[1.0, 2.0], 5).is_empty());
    }

    #[test]
    fn bulk_truncate_stacks_rows() {
        let rows = vec![vec![1.0_f32, 0.0, 9.0], vec![0.0, 1.0, 9.0]];
        let out = truncate_rows(&rows, 2);
        assert_eq!(out.len(), 4);
        assert!((out[0] - 1.0).abs() < 1e-5);
        assert!((out[3] - 1.0).abs() < 1e-5);
    }
}
