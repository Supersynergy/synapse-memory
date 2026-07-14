//! NEON int8 dot product using stable aarch64 intrinsics.
//! Strategy: vmull_s8 (i8×i8→i16) + vpaddlq_s16 (i16→i32 horizontal add)
//! 16 elements per inner iteration via two vmull_s8 on low/high halves of int8x16.
//! Fallback: 4-way unrolled scalar i32 accumulator.

/// Scalar fallback — 4-way unrolled.
#[inline]
pub fn dot_i8_scalar(a: &[i8], b: &[i8]) -> i32 {
    debug_assert_eq!(a.len(), b.len());
    let n = a.len();
    let mut acc0: i32 = 0;
    let mut acc1: i32 = 0;
    let mut acc2: i32 = 0;
    let mut acc3: i32 = 0;
    let mut i = 0;
    while i + 4 <= n {
        unsafe {
            acc0 += *a.get_unchecked(i) as i32 * *b.get_unchecked(i) as i32;
            acc1 += *a.get_unchecked(i + 1) as i32 * *b.get_unchecked(i + 1) as i32;
            acc2 += *a.get_unchecked(i + 2) as i32 * *b.get_unchecked(i + 2) as i32;
            acc3 += *a.get_unchecked(i + 3) as i32 * *b.get_unchecked(i + 3) as i32;
        }
        i += 4;
    }
    while i < n {
        unsafe {
            acc0 += *a.get_unchecked(i) as i32 * *b.get_unchecked(i) as i32;
        }
        i += 1;
    }
    (acc0 + acc1) + (acc2 + acc3)
}

/// NEON vmull_s8 + vpaddlq_s16: stable on aarch64, ~4-8× scalar for dim≥128.
/// Each loop iteration: 2 × int8x8 → int16x8 (vmull_s8), then widen to i32.
/// 64-byte stride: 4 pairs of low/high halves.
#[cfg(all(target_arch = "aarch64", feature = "neon"))]
#[inline]
pub fn dot_i8_neon(a: &[i8], b: &[i8]) -> i32 {
    use std::arch::aarch64::*;
    debug_assert_eq!(a.len(), b.len());

    let n = a.len();
    let mut i = 0;
    // int32x4 accumulator
    let mut acc: int32x4_t = unsafe { vdupq_n_s32(0) };

    // Process 16 elements per iteration
    while i + 16 <= n {
        unsafe {
            let ap = a.as_ptr().add(i);
            let bp = b.as_ptr().add(i);
            // load int8x16
            let av = vld1q_s8(ap);
            let bv = vld1q_s8(bp);
            // low halves: int8x8 → int16x8
            let prod_lo: int16x8_t = vmull_s8(vget_low_s8(av), vget_low_s8(bv));
            // high halves: int8x8 → int16x8
            let prod_hi: int16x8_t = vmull_s8(vget_high_s8(av), vget_high_s8(bv));
            // widen i16→i32 and accumulate
            acc = vpadalq_s16(acc, prod_lo);
            acc = vpadalq_s16(acc, prod_hi);
        }
        i += 16;
    }

    // horizontal sum of acc
    let mut result: i32 = unsafe { vaddvq_s32(acc) };

    // scalar tail
    while i < n {
        unsafe {
            result += *a.get_unchecked(i) as i32 * *b.get_unchecked(i) as i32;
        }
        i += 1;
    }
    result
}

/// Public dispatch: NEON when feature=neon + aarch64, else scalar.
#[inline]
pub fn dot_i8(a: &[i8], b: &[i8]) -> i32 {
    #[cfg(all(target_arch = "aarch64", feature = "neon"))]
    {
        dot_i8_neon(a, b)
    }
    #[cfg(not(all(target_arch = "aarch64", feature = "neon")))]
    {
        dot_i8_scalar(a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correctness_small() {
        let a = vec![1i8, 2, 3, 4];
        let b = vec![4i8, 3, 2, 1];
        assert_eq!(dot_i8(&a, &b), 20);
        assert_eq!(dot_i8_scalar(&a, &b), 20);
    }

    #[test]
    fn correctness_dim128() {
        let a: Vec<i8> = (0..128).map(|i: i32| (i % 127) as i8).collect();
        let b: Vec<i8> = (0..128).map(|i: i32| ((127 - i) % 127) as i8).collect();
        let scalar = dot_i8_scalar(&a, &b);
        let neon = dot_i8(&a, &b);
        assert_eq!(scalar, neon, "scalar={scalar} neon={neon}");
    }

    #[test]
    fn correctness_dim256() {
        let a: Vec<i8> = (0i32..256).map(|i| (i % 100 - 50) as i8).collect();
        let b: Vec<i8> = (0i32..256).map(|i| ((i * 3) % 100 - 50) as i8).collect();
        let scalar = dot_i8_scalar(&a, &b);
        let neon = dot_i8(&a, &b);
        assert_eq!(scalar, neon, "scalar={scalar} neon={neon}");
    }

    #[test]
    fn negative_values() {
        let a = vec![-1i8, -2, 3, -4, 5, -6, 7, -8, 1, 2, 3, 4, 5, 6, 7, 8];
        let b = vec![8i8, 7, 6, 5, 4, 3, 2, 1, 8, 7, 6, 5, 4, 3, 2, 1];
        let scalar = dot_i8_scalar(&a, &b);
        let neon = dot_i8(&a, &b);
        assert_eq!(scalar, neon);
    }

    #[test]
    fn non_multiple_of_16() {
        let a: Vec<i8> = (0..37).map(|i: i32| (i % 50 - 25) as i8).collect();
        let b: Vec<i8> = (0..37).map(|i: i32| ((i * 2) % 50 - 25) as i8).collect();
        assert_eq!(dot_i8_scalar(&a, &b), dot_i8(&a, &b));
    }
}
