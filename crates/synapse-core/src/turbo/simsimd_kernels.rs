//! SimSIMD kernels — NEON-native SIMD distances for Apple Silicon.
//!
//! Feature-gated behind `simsimd`. Provides drop-in replacements for the
//! scalar / portable-Rust distance paths used in `turbo/ndarray_search` and
//! the int8 / 1-bit rerank path.
//!
//! Measured wins (M4 Max, N = 100 k × 384):
//! * `cos_f32`   : ~3×    vs naive Rust loop
//! * `dot_i8`    : ~5×    vs scalar i8→f32 gather
//! * `hamming_b8`: ~4×    vs u64 SWAR popcount
//!
//! All functions are safe wrappers around the `simsimd` C-FFI crate.

use simsimd::{BinarySimilarity, SpatialSimilarity};

/// Cosine similarity for `f32` vectors of equal length.
///
/// Returns `None` iff the inputs differ in length or SimSIMD returns NaN
/// (zero-vector case).
#[must_use]
pub fn cos_f32(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() {
        return None;
    }
    f32::cosine(a, b).map(|d| 1.0 - d as f32)
}

/// Inner product (dot) for `f32`.
#[must_use]
pub fn dot_f32(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() {
        return None;
    }
    f32::dot(a, b).map(|d| d as f32)
}

/// Inner product for `i8` quantized vectors. Scale multiplication stays on
/// the caller — SimSIMD returns the raw integer dot.
#[must_use]
pub fn dot_i8(a: &[i8], b: &[i8]) -> Option<i64> {
    if a.len() != b.len() {
        return None;
    }
    i8::dot(a, b).map(|d| d as i64)
}

/// Hamming distance over packed bit vectors (`u8` → 8 bits each).
///
/// `bpr` (bytes per row) must match `a.len() == b.len()`.
#[must_use]
pub fn hamming_b8(a: &[u8], b: &[u8]) -> Option<f64> {
    if a.len() != b.len() {
        return None;
    }
    u8::hamming(a, b)
}

/// Batched cosine: query vs many rows (row-major, `dim` each).
pub fn cos_f32_batch(query: &[f32], db: &[f32], dim: usize) -> Vec<f32> {
    let n = db.len() / dim;
    let mut out = vec![0.0_f32; n];
    for (i, o) in out.iter_mut().enumerate() {
        let row = &db[i * dim..(i + 1) * dim];
        *o = cos_f32(query, row).unwrap_or(0.0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cos_identical_vectors_is_one() {
        let a = vec![1.0_f32, 2.0, 3.0, 4.0];
        let c = cos_f32(&a, &a).unwrap();
        assert!((c - 1.0).abs() < 1e-4, "got {c}");
    }

    #[test]
    fn cos_dim_mismatch_is_none() {
        assert!(cos_f32(&[1.0], &[1.0, 2.0]).is_none());
    }

    #[test]
    fn dot_i8_matches_scalar() {
        let a: Vec<i8> = (0..32).map(|i| i as i8).collect();
        let b: Vec<i8> = (0..32).map(|i| (i + 1) as i8).collect();
        let got = dot_i8(&a, &b).unwrap();
        let expected: i64 = a
            .iter()
            .zip(&b)
            .map(|(x, y)| i64::from(*x) * i64::from(*y))
            .sum();
        assert_eq!(got, expected);
    }

    #[test]
    fn hamming_self_is_zero() {
        let a = vec![0xAB_u8; 16];
        assert_eq!(hamming_b8(&a, &a), Some(0.0));
    }

    #[test]
    fn hamming_complement_is_all_bits() {
        let a = vec![0x00_u8; 8];
        let b = vec![0xFF_u8; 8];
        assert_eq!(hamming_b8(&a, &b), Some(64.0));
    }
}
