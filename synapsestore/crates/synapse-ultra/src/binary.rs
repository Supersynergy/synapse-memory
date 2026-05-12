/// Pack 384 f32 signs into 48 bytes (384 bits).
pub fn pack_signs(v: &[f32]) -> Vec<u8> {
    assert_eq!(v.len(), 384);
    let mut out = vec![0u8; 48];
    for (i, &x) in v.iter().enumerate() {
        if x >= 0.0 {
            out[i / 8] |= 1 << (i % 8);
        }
    }
    out
}

/// Hamming distance between two 48-byte bit vectors. Uses u64 chunks
/// (6 popcnt vs 48 byte ops). For 48 bytes: 6×u64 + 0×u8 tail.
#[inline]
pub fn hamming_distance(a: &[u8], b: &[u8]) -> u32 {
    debug_assert_eq!(a.len(), 48);
    debug_assert_eq!(b.len(), 48);
    let pa = a.as_ptr() as *const u64;
    let pb = b.as_ptr() as *const u64;
    // SAFETY: 48 bytes = 6×u64 fits, alignment OK because read_unaligned.
    unsafe {
        let mut sum: u32 = 0;
        for i in 0..6 {
            sum += (pa.add(i).read_unaligned() ^ pb.add(i).read_unaligned()).count_ones();
        }
        sum
    }
}

/// Build packed-sign matrix from row-major f32 matrix.
pub fn build_binary_matrix(matrix: &ndarray::Array2<f32>) -> Vec<u8> {
    let n = matrix.nrows();
    let mut out = vec![0u8; n * 48];
    for i in 0..n {
        let row = matrix.row(i);
        let packed = pack_signs(row.as_slice().unwrap());
        out[i * 48..(i + 1) * 48].copy_from_slice(&packed);
    }
    out
}

use std::sync::OnceLock;

/// Fixed 384×384 rotation matrix, seeded once at startup.
/// MUST be the same for build_binary_matrix_rotated and pack_signs_rotated.
static ROTATION: OnceLock<Vec<f32>> = OnceLock::new();

fn get_rotation_matrix(dim: usize) -> &'static Vec<f32> {
    ROTATION.get_or_init(|| {
        let seed: u64 = 0xDEAD_BEEF_CAFE_1337;
        let mut rng_state = seed;
        let n = dim * dim;
        let mut mat: Vec<f32> = (0..n)
            .map(|_| {
                rng_state ^= rng_state << 13;
                rng_state ^= rng_state >> 7;
                rng_state ^= rng_state << 17;
                (rng_state as i64 as f32) / (i64::MAX as f32)
            })
            .collect();
        // Gram-Schmidt orthogonalize columns (one-time cost, ~384^3 ops)
        for i in 0..dim {
            // normalize column i
            let norm: f32 = (0..dim)
                .map(|k| mat[i * dim + k] * mat[i * dim + k])
                .sum::<f32>()
                .sqrt();
            if norm > 1e-10 {
                for k in 0..dim {
                    mat[i * dim + k] /= norm;
                }
            }
            // subtract projection from subsequent columns
            for j in (i + 1)..dim {
                let dot: f32 = (0..dim).map(|k| mat[i * dim + k] * mat[j * dim + k]).sum();
                for k in 0..dim {
                    mat[j * dim + k] -= dot * mat[i * dim + k];
                }
            }
        }
        mat
    })
}

/// RaBitQ: apply fixed seeded rotation R, then sign-quantize.
pub fn pack_signs_rotated(v: &[f32]) -> Vec<u8> {
    let dim = v.len();
    assert_eq!(dim, 384);
    let r = get_rotation_matrix(dim);
    let rotated: Vec<f32> = (0..dim)
        .map(|i| (0..dim).map(|j| r[i * dim + j] * v[j]).sum())
        .collect();
    pack_signs(&rotated)
}

/// Build binary matrix with RaBitQ rotation applied.
pub fn build_binary_matrix_rotated(matrix: &ndarray::Array2<f32>) -> Vec<u8> {
    let n = matrix.nrows();
    let mut out = vec![0u8; n * 48];
    for i in 0..n {
        let row = matrix.row(i);
        let packed = pack_signs_rotated(row.as_slice().unwrap());
        out[i * 48..(i + 1) * 48].copy_from_slice(&packed);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pack_roundtrip_all_positive() {
        let v = vec![1.0f32; 384];
        let packed = pack_signs(&v);
        assert_eq!(packed.len(), 48);
        assert!(packed.iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn test_hamming_identical() {
        let v = vec![0.5f32; 384];
        let p = pack_signs(&v);
        assert_eq!(hamming_distance(&p, &p), 0);
    }

    #[test]
    fn test_hamming_opposite() {
        let pos = vec![1.0f32; 384];
        let neg = vec![-1.0f32; 384];
        let pp = pack_signs(&pos);
        let pn = pack_signs(&neg);
        assert_eq!(hamming_distance(&pp, &pn), 384);
    }

    #[test]
    fn test_recall_rotated_vs_plain() {
        // Generate 1000 pseudo-random 384-d vectors
        let mut state: u64 = 0x1234_5678_9ABC_DEF0;
        let mut next = || -> f32 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state as i64 as f32) / (i64::MAX as f32)
        };

        let n = 1000usize;
        let dim = 384usize;
        let vecs: Vec<Vec<f32>> = (0..n).map(|_| (0..dim).map(|_| next()).collect()).collect();

        // Normalize
        let vecs: Vec<Vec<f32>> = vecs
            .into_iter()
            .map(|v| {
                let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                v.into_iter().map(|x| x / norm).collect()
            })
            .collect();

        // Build packed matrices
        let plain: Vec<Vec<u8>> = vecs.iter().map(|v| pack_signs(v)).collect();
        let rotated: Vec<Vec<u8>> = vecs.iter().map(|v| pack_signs_rotated(v)).collect();

        let k = 10usize;
        let queries: Vec<usize> = (0..10).map(|i| i * 97).collect();

        let mut plain_recall_total = 0usize;
        let mut rot_recall_total = 0usize;

        for &qi in &queries {
            let qv = &vecs[qi];

            // Ground truth: top-k by dot product (vectors are normalized)
            let mut scores: Vec<(usize, f32)> = vecs
                .iter()
                .enumerate()
                .map(|(i, v)| (i, v.iter().zip(qv).map(|(a, b)| a * b).sum::<f32>()))
                .collect();
            scores.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            let gt: std::collections::HashSet<usize> =
                scores[..k].iter().map(|(i, _)| *i).collect();

            // Plain hamming top-k
            let mut ph: Vec<(usize, u32)> = plain
                .iter()
                .enumerate()
                .map(|(i, p)| (i, hamming_distance(&plain[qi], p)))
                .collect();
            ph.sort_unstable_by_key(|x| x.1);
            let plain_hits: std::collections::HashSet<usize> =
                ph[..k].iter().map(|(i, _)| *i).collect();

            // Rotated hamming top-k
            let mut rh: Vec<(usize, u32)> = rotated
                .iter()
                .enumerate()
                .map(|(i, r)| (i, hamming_distance(&rotated[qi], r)))
                .collect();
            rh.sort_unstable_by_key(|x| x.1);
            let rot_hits: std::collections::HashSet<usize> =
                rh[..k].iter().map(|(i, _)| *i).collect();

            plain_recall_total += plain_hits.intersection(&gt).count();
            rot_recall_total += rot_hits.intersection(&gt).count();
        }

        let plain_recall = plain_recall_total as f32 / (queries.len() * k) as f32;
        let rot_recall = rot_recall_total as f32 / (queries.len() * k) as f32;

        println!("plain recall@{k}: {plain_recall:.3}  rotated recall@{k}: {rot_recall:.3}");
        // Both methods should achieve meaningful recall on random data.
        // RaBitQ gains appear at scale (100k+); here we verify correctness, not superiority.
        assert!(plain_recall > 0.1, "plain recall too low: {plain_recall}");
        assert!(rot_recall > 0.1, "rotated recall too low: {rot_recall}");
    }
}
