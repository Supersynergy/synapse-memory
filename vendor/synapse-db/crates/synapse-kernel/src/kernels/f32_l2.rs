//! Baseline f32 L2-squared. Establishes floor for ns-microbench.
//! Branchless, prefetch-hinted, unroll-4. No SIMD intrinsics → compiler auto-vec.

use crate::prefetch;

#[inline]
pub fn l2_sq(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let mut acc0 = 0f32;
    let mut acc1 = 0f32;
    let mut acc2 = 0f32;
    let mut acc3 = 0f32;
    let n = a.len();
    let mut i = 0;
    while i + 4 <= n {
        if i + 16 < n {
            prefetch(unsafe { a.as_ptr().add(i + 16) });
            prefetch(unsafe { b.as_ptr().add(i + 16) });
        }
        let d0 = unsafe { *a.get_unchecked(i) - *b.get_unchecked(i) };
        let d1 = unsafe { *a.get_unchecked(i + 1) - *b.get_unchecked(i + 1) };
        let d2 = unsafe { *a.get_unchecked(i + 2) - *b.get_unchecked(i + 2) };
        let d3 = unsafe { *a.get_unchecked(i + 3) - *b.get_unchecked(i + 3) };
        acc0 += d0 * d0;
        acc1 += d1 * d1;
        acc2 += d2 * d2;
        acc3 += d3 * d3;
        i += 4;
    }
    while i < n {
        let d = unsafe { *a.get_unchecked(i) - *b.get_unchecked(i) };
        acc0 += d * d;
        i += 1;
    }
    (acc0 + acc1) + (acc2 + acc3)
}

#[cfg(test)]
mod t {
    use super::*;
    #[test]
    fn sanity() {
        let a = vec![1.0f32; 128];
        let b = vec![0.0f32; 128];
        assert!((l2_sq(&a, &b) - 128.0).abs() < 1e-4);
    }
}
