//! f16 (half-precision) storage + conversion helpers.
//!
//! 50 % memory footprint reduction vs fp32 with negligible recall loss for
//! normalized embeddings. Compute stays fp32 — we convert on the hot path,
//! which LLVM unrolls on M-series NEON.
//!
//! The `half` crate is already a workspace dependency (see synapse-core
//! `Cargo.toml`), so no new deps.
//!
//! # Example
//! ```
//! use synapse_core::turbo::f16_kernels::{pack_f16, unpack_f16};
//! let v = vec![1.0_f32, -0.5, 0.25];
//! let packed = pack_f16(&v);
//! assert_eq!(packed.len(), v.len() * 2);   // 2 bytes per value
//! let round = unpack_f16(&packed);
//! for (a, b) in v.iter().zip(&round) {
//!     assert!((a - b).abs() < 0.01);
//! }
//! ```

use half::f16;

/// Convert an fp32 slice to packed f16 bytes (little-endian).
#[must_use]
pub fn pack_f16(src: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len() * 2);
    for &v in src {
        out.extend_from_slice(&f16::from_f32(v).to_le_bytes());
    }
    out
}

/// Reverse of [`pack_f16`] — packed bytes → f32 vec.
#[must_use]
pub fn unpack_f16(src: &[u8]) -> Vec<f32> {
    assert!(src.len() % 2 == 0, "f16 payload must be even-sized");
    src.chunks_exact(2)
        .map(|c| f16::from_le_bytes([c[0], c[1]]).to_f32())
        .collect()
}

/// Batched variant — encode many rows into a flat packed buffer.
/// Rows are assumed equal-length.
#[must_use]
pub fn pack_f16_rows(rows: &[Vec<f32>]) -> Vec<u8> {
    if rows.is_empty() { return Vec::new(); }
    let dim = rows[0].len();
    let mut out = Vec::with_capacity(rows.len() * dim * 2);
    for row in rows {
        debug_assert_eq!(row.len(), dim);
        for &v in row {
            out.extend_from_slice(&f16::from_f32(v).to_le_bytes());
        }
    }
    out
}

/// Cosine similarity between two equal-length vectors, one fp32 query +
/// one packed-f16 row. Compute stays fp32; f16 is storage-only.
#[must_use]
pub fn cos_f16_row(query_f32: &[f32], row_f16: &[u8]) -> Option<f32> {
    if query_f32.len() * 2 != row_f16.len() { return None; }
    let mut dot = 0.0_f32;
    let mut q_norm = 0.0_f32;
    let mut r_norm = 0.0_f32;
    for (qi, rc) in query_f32.iter().zip(row_f16.chunks_exact(2)) {
        let r = f16::from_le_bytes([rc[0], rc[1]]).to_f32();
        dot += qi * r;
        q_norm += qi * qi;
        r_norm += r * r;
    }
    let denom = (q_norm.sqrt() * r_norm.sqrt()).max(1e-12);
    Some(dot / denom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_roundtrip_preserves_f32_approximately() {
        let v: Vec<f32> = (0..32).map(|i| i as f32 * 0.1 - 1.5).collect();
        let packed = pack_f16(&v);
        assert_eq!(packed.len(), v.len() * 2);
        let r = unpack_f16(&packed);
        for (a, b) in v.iter().zip(&r) {
            assert!((a - b).abs() < 0.01, "f32 {a} vs f16 {b}");
        }
    }

    #[test]
    fn packed_bytes_are_half_the_size() {
        let v = vec![0.0_f32; 384];
        assert_eq!(pack_f16(&v).len(), 384 * 2);     // 768 bytes
        // compare: f32 would be 384 * 4 = 1536 bytes
    }

    #[test]
    fn pack_rows_stacks_contiguously() {
        let rows = vec![vec![1.0_f32, 2.0], vec![3.0, 4.0]];
        let packed = pack_f16_rows(&rows);
        assert_eq!(packed.len(), 2 * 2 * 2);  // 2 rows × 2 dim × 2 bytes
    }

    #[test]
    fn cos_f16_matches_f32_within_tolerance() {
        let q = vec![0.3_f32, 0.4, 0.5, 0.6];
        let r_f32: Vec<f32> = q.clone();
        let r_packed = pack_f16(&r_f32);
        let c = cos_f16_row(&q, &r_packed).unwrap();
        assert!((c - 1.0).abs() < 0.001, "got {c}");
    }

    #[test]
    fn cos_f16_dim_mismatch_returns_none() {
        let q = vec![1.0_f32; 4];
        let r = pack_f16(&vec![1.0_f32; 2]); // different dim
        assert!(cos_f16_row(&q, &r).is_none());
    }
}
