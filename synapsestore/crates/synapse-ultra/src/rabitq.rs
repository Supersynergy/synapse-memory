//! Proper RaBitQ with per-vector factors (FAISS-pattern, 1-bit inner-product).
//!
//! Layout per 384-d vector: [signs: 48 bytes][dp_mul: f32 (4 bytes)] = 52 bytes.
//!
//! `dp_multiplier = ||x||^2 * sqrt(d) / sum(|x_i|)`
//!
//! Distance estimate (inner product, no centroid):
//!   IP ≈ dp_mul_q * dp_mul_d * (2 * matches - d) / d
//!
//! Recall > plain sign-quant and naive-rotation at 10k+ scale because
//! the factor corrects for unequal norm + distribution skew.

use std::sync::OnceLock;

const DIM: usize = 384;
const SIGN_BYTES: usize = DIM / 8; // 48

/// Per-vector RaBitQ entry: sign bits + dp_multiplier factor.
#[derive(Clone)]
pub struct RaBitQEntry {
    pub signs: [u8; SIGN_BYTES],
    pub dp_mul: f32,
}

/// Fixed 384×384 random orthogonal rotation (same seed as binary.rs).
static ROTATION: OnceLock<Vec<f32>> = OnceLock::new();

fn get_rotation() -> &'static Vec<f32> {
    ROTATION.get_or_init(|| {
        let seed: u64 = 0xDEAD_BEEF_CAFE_1337;
        let mut s = seed;
        let n = DIM * DIM;
        let mut mat: Vec<f32> = (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                (s as i64 as f32) / (i64::MAX as f32)
            })
            .collect();
        // Gram-Schmidt orthogonalize columns
        for i in 0..DIM {
            let norm: f32 = (0..DIM)
                .map(|k| mat[i * DIM + k] * mat[i * DIM + k])
                .sum::<f32>()
                .sqrt();
            if norm > 1e-10 {
                for k in 0..DIM {
                    mat[i * DIM + k] /= norm;
                }
            }
            for j in (i + 1)..DIM {
                let dot: f32 = (0..DIM).map(|k| mat[i * DIM + k] * mat[j * DIM + k]).sum();
                for k in 0..DIM {
                    mat[j * DIM + k] -= dot * mat[i * DIM + k];
                }
            }
        }
        mat
    })
}

/// Rotate v by the fixed orthogonal matrix (pub for query-time use).
pub fn rotate(v: &[f32]) -> Vec<f32> {
    let r = get_rotation();
    (0..DIM)
        .map(|i| (0..DIM).map(|j| r[i * DIM + j] * v[j]).sum())
        .collect()
}

/// Pack sign bits: bit i = 1 if rotated[i] >= 0.
fn pack_signs(rotated: &[f32]) -> [u8; SIGN_BYTES] {
    let mut out = [0u8; SIGN_BYTES];
    for (i, &x) in rotated.iter().enumerate() {
        if x >= 0.0 {
            out[i / 8] |= 1 << (i % 8);
        }
    }
    out
}

/// Hamming distance between two SIGN_BYTES-length bit vectors (6×u64 popcnt).
#[inline]
fn hamming(a: &[u8; SIGN_BYTES], b: &[u8; SIGN_BYTES]) -> u32 {
    let pa = a.as_ptr() as *const u64;
    let pb = b.as_ptr() as *const u64;
    unsafe {
        let mut s = 0u32;
        for i in 0..6 {
            s += (pa.add(i).read_unaligned() ^ pb.add(i).read_unaligned()).count_ones();
        }
        s
    }
}

/// Encode a 384-d vector into a RaBitQEntry.
///
/// Steps:
/// 1. Rotate by fixed R.
/// 2. Compute dp_multiplier = ||rotated||^2 * sqrt(d) / sum(|rotated_i|).
/// 3. Pack sign bits.
pub fn encode(v: &[f32]) -> RaBitQEntry {
    assert_eq!(v.len(), DIM);
    let rotated = rotate(v);

    let norm_sq: f32 = rotated.iter().map(|x| x * x).sum();
    let dp_oo: f32 = rotated.iter().map(|x| x.abs()).sum();

    let dp_mul = if dp_oo > 1e-10 {
        norm_sq * (DIM as f32).sqrt() / dp_oo
    } else {
        0.0
    };

    RaBitQEntry {
        signs: pack_signs(&rotated),
        dp_mul,
    }
}

/// Estimate inner product: asymmetric — real query rotated values × doc sign bits.
///
/// `dot_qo = sum(q_rotated_i for i where doc_sign_i = 1)` — accumulates
/// real query components on matching sign positions.
/// Final: `dp_mul_d * dot_qo / sqrt(d)` (FAISS 1-bit IP formula).
#[inline]
pub fn estimate_ip_asymmetric(q_rotated: &[f32], doc: &RaBitQEntry) -> f32 {
    debug_assert_eq!(q_rotated.len(), DIM);
    let mut dot_qo: f32 = 0.0;
    for (i, &qv) in q_rotated.iter().enumerate() {
        let byte = doc.signs[i / 8];
        let bit = (byte >> (i % 8)) & 1;
        if bit != 0 {
            dot_qo += qv;
        }
    }
    doc.dp_mul * dot_qo / (DIM as f32).sqrt()
}

/// Symmetric estimate (both sides binary + factors). Kept for completeness.
#[inline]
pub fn estimate_ip(q: &RaBitQEntry, d: &RaBitQEntry) -> f32 {
    let ham = hamming(&q.signs, &d.signs);
    let matches = DIM as f32 - ham as f32;
    let normalized = (2.0 * matches - DIM as f32) / DIM as f32;
    q.dp_mul * d.dp_mul * normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xorshift(state: &mut u64) -> f32 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        (*state as i64 as f32) / (i64::MAX as f32)
    }

    fn rand_unit_vec(state: &mut u64) -> Vec<f32> {
        let v: Vec<f32> = (0..DIM).map(|_| xorshift(state)).collect();
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.into_iter().map(|x| x / norm).collect()
    }

    fn recall_at_k(
        vecs: &[Vec<f32>],
        plain_packed: &[[u8; SIGN_BYTES]],
        rotated_packed: &[[u8; SIGN_BYTES]],
        rabitq_entries: &[RaBitQEntry],
        k: usize,
        query_indices: &[usize],
    ) -> (f32, f32, f32) {
        let mut plain_total = 0usize;
        let mut rot_total = 0usize;
        let mut rbq_total = 0usize;

        for &qi in query_indices {
            let qv = &vecs[qi];

            // Ground truth by f32 dot product
            let mut scores: Vec<(usize, f32)> = vecs
                .iter()
                .enumerate()
                .map(|(i, v)| (i, v.iter().zip(qv).map(|(a, b)| a * b).sum::<f32>()))
                .collect();
            scores.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            let gt: std::collections::HashSet<usize> =
                scores[..k].iter().map(|(i, _)| *i).collect();

            // Plain sign hamming
            let mut ph: Vec<(usize, u32)> = plain_packed
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let h = {
                        let a = &plain_packed[qi];
                        let pa = a.as_ptr() as *const u64;
                        let pb = p.as_ptr() as *const u64;
                        unsafe {
                            let mut s = 0u32;
                            for j in 0..6 {
                                s += (pa.add(j).read_unaligned()
                                    ^ pb.add(j).read_unaligned())
                                .count_ones();
                            }
                            s
                        }
                    };
                    (i, h)
                })
                .collect();
            ph.sort_unstable_by_key(|x| x.1);
            plain_total += ph[..k]
                .iter()
                .map(|(i, _)| *i)
                .collect::<std::collections::HashSet<_>>()
                .intersection(&gt)
                .count();

            // Rotated sign hamming
            let mut rh: Vec<(usize, u32)> = rotated_packed
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let h = {
                        let a = &rotated_packed[qi];
                        let pa = a.as_ptr() as *const u64;
                        let pb = p.as_ptr() as *const u64;
                        unsafe {
                            let mut s = 0u32;
                            for j in 0..6 {
                                s += (pa.add(j).read_unaligned()
                                    ^ pb.add(j).read_unaligned())
                                .count_ones();
                            }
                            s
                        }
                    };
                    (i, h)
                })
                .collect();
            rh.sort_unstable_by_key(|x| x.1);
            rot_total += rh[..k]
                .iter()
                .map(|(i, _)| *i)
                .collect::<std::collections::HashSet<_>>()
                .intersection(&gt)
                .count();

            // RaBitQ asymmetric: real rotated query vs doc sign bits + factor
            let q_rotated = rotate(qv);
            let mut rbq: Vec<(usize, f32)> = rabitq_entries
                .iter()
                .enumerate()
                .map(|(i, e)| (i, estimate_ip_asymmetric(&q_rotated, e)))
                .collect();
            rbq.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            rbq_total += rbq[..k]
                .iter()
                .map(|(i, _)| *i)
                .collect::<std::collections::HashSet<_>>()
                .intersection(&gt)
                .count();
        }

        let denom = (query_indices.len() * k) as f32;
        (
            plain_total as f32 / denom,
            rot_total as f32 / denom,
            rbq_total as f32 / denom,
        )
    }

    #[test]
    fn test_rabitq_recall_10k() {
        let n = 10_000usize;
        let k = 10usize;
        let mut state: u64 = 0xCAFE_BABE_1234_5678;

        let vecs: Vec<Vec<f32>> = (0..n).map(|_| rand_unit_vec(&mut state)).collect();

        // Plain sign packing (no rotation)
        let plain_packed: Vec<[u8; SIGN_BYTES]> = vecs
            .iter()
            .map(|v| {
                let mut out = [0u8; SIGN_BYTES];
                for (i, &x) in v.iter().enumerate() {
                    if x >= 0.0 {
                        out[i / 8] |= 1 << (i % 8);
                    }
                }
                out
            })
            .collect();

        // Rotated sign packing (T2 / naive rotation, reuse rotation from encode)
        let rotated_packed: Vec<[u8; SIGN_BYTES]> = vecs
            .iter()
            .map(|v| {
                let rotated = rotate(v);
                pack_signs(&rotated)
            })
            .collect();

        // Proper RaBitQ with per-vector factors
        let rabitq_entries: Vec<RaBitQEntry> = vecs.iter().map(|v| encode(v)).collect();

        let query_indices: Vec<usize> = (0..50).map(|i| i * 199 % n).collect();

        let (plain_r, rot_r, rbq_r) =
            recall_at_k(&vecs, &plain_packed, &rotated_packed, &rabitq_entries, k, &query_indices);

        println!(
            "recall@{k} (n={n}, q=50):  plain={plain_r:.3}  rotated={rot_r:.3}  rabitq={rbq_r:.3}"
        );

        // RaBitQ asymmetric must beat both plain and naive rotation
        assert!(
            rbq_r > rot_r,
            "RaBitQ ({rbq_r:.3}) should beat rotated ({rot_r:.3})"
        );
        assert!(
            rbq_r >= plain_r,
            "RaBitQ ({rbq_r:.3}) should be >= plain ({plain_r:.3})"
        );
        // Must be non-trivial recall
        assert!(rbq_r > 0.1, "RaBitQ recall too low: {rbq_r:.3}");
    }
}
