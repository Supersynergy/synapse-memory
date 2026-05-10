//! Binary Hamming via XOR + count_ones. Target: <0.5 ns / 256-bit vec.
//! Compiler maps count_ones() → CNT (NEON) on aarch64; competitive with hand-asm.

#[inline]
pub fn hamming_u64(a: &[u64], b: &[u64]) -> u32 {
    debug_assert_eq!(a.len(), b.len());
    let mut acc = 0u32;
    let n = a.len();
    let mut i = 0;
    while i + 4 <= n {
        acc += (a[i]   ^ b[i]  ).count_ones();
        acc += (a[i+1] ^ b[i+1]).count_ones();
        acc += (a[i+2] ^ b[i+2]).count_ones();
        acc += (a[i+3] ^ b[i+3]).count_ones();
        i += 4;
    }
    while i < n {
        acc += (a[i] ^ b[i]).count_ones();
        i += 1;
    }
    acc
}

#[cfg(test)]
mod t {
    use super::*;
    #[test]
    fn distinct_full() {
        let a = vec![0u64; 4]; let b = vec![!0u64; 4];
        assert_eq!(hamming_u64(&a, &b), 256);
    }
}
