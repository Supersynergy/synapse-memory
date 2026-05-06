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
}
