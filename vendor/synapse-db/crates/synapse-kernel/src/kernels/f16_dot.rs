/// NEON f16 dot product using stable aarch64 intrinsics.
/// Strategy: load u16 bits → transmute to float16x4_t → vcvt_f32_f16 (4 f16→f32x4) → vmlaq_f32 FMA.
/// 8 elements per iteration (2×4). Upcast to f32 accumulator avoids f16 overflow.
/// float16x4_t + vcvt_f32_f16 are stable (no stdarch_neon_f16 gate).
/// Scalar fallback: f32 accumulator via half::f16::to_f32().
use half::f16;

/// Scalar fallback — 4-way unrolled.
#[inline]
pub fn dot_f16_scalar(a: &[f16], b: &[f16]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let n = a.len();
    let mut acc0 = 0f32;
    let mut acc1 = 0f32;
    let mut acc2 = 0f32;
    let mut acc3 = 0f32;
    let mut i = 0;
    while i + 4 <= n {
        unsafe {
            acc0 += a.get_unchecked(i).to_f32() * b.get_unchecked(i).to_f32();
            acc1 += a.get_unchecked(i + 1).to_f32() * b.get_unchecked(i + 1).to_f32();
            acc2 += a.get_unchecked(i + 2).to_f32() * b.get_unchecked(i + 2).to_f32();
            acc3 += a.get_unchecked(i + 3).to_f32() * b.get_unchecked(i + 3).to_f32();
        }
        i += 4;
    }
    while i < n {
        unsafe {
            acc0 += a.get_unchecked(i).to_f32() * b.get_unchecked(i).to_f32();
        }
        i += 1;
    }
    (acc0 + acc1) + (acc2 + acc3)
}

/// NEON: transmute u16→float16x4_t, vcvt_f32_f16 (stable), then vmlaq_f32.
/// 16 f16 per main loop (4 accumulators), 8-wide tail.
#[cfg(all(target_arch = "aarch64", feature = "neon"))]
#[inline]
pub fn dot_f16_neon(a: &[f16], b: &[f16]) -> f32 {
    use std::arch::aarch64::*;
    debug_assert_eq!(a.len(), b.len());

    let n = a.len();
    let mut i = 0;
    let mut acc0: float32x4_t = unsafe { vdupq_n_f32(0.0) };
    let mut acc1: float32x4_t = unsafe { vdupq_n_f32(0.0) };

    let mut acc2: float32x4_t = unsafe { vdupq_n_f32(0.0) };
    let mut acc3: float32x4_t = unsafe { vdupq_n_f32(0.0) };

    // 16 f16 per iteration: 4 × 4-lane chunks — doubles throughput vs 8-wide
    while i + 16 <= n {
        unsafe {
            let ap = a.as_ptr().add(i) as *const u16;
            let bp = b.as_ptr().add(i) as *const u16;

            let au0: uint16x4_t = vld1_u16(ap);
            let bu0: uint16x4_t = vld1_u16(bp);
            let au1: uint16x4_t = vld1_u16(ap.add(4));
            let bu1: uint16x4_t = vld1_u16(bp.add(4));
            let au2: uint16x4_t = vld1_u16(ap.add(8));
            let bu2: uint16x4_t = vld1_u16(bp.add(8));
            let au3: uint16x4_t = vld1_u16(ap.add(12));
            let bu3: uint16x4_t = vld1_u16(bp.add(12));

            let af0: float32x4_t =
                vcvt_f32_f16(std::mem::transmute::<uint16x4_t, float16x4_t>(au0));
            let bf0: float32x4_t =
                vcvt_f32_f16(std::mem::transmute::<uint16x4_t, float16x4_t>(bu0));
            let af1: float32x4_t =
                vcvt_f32_f16(std::mem::transmute::<uint16x4_t, float16x4_t>(au1));
            let bf1: float32x4_t =
                vcvt_f32_f16(std::mem::transmute::<uint16x4_t, float16x4_t>(bu1));
            let af2: float32x4_t =
                vcvt_f32_f16(std::mem::transmute::<uint16x4_t, float16x4_t>(au2));
            let bf2: float32x4_t =
                vcvt_f32_f16(std::mem::transmute::<uint16x4_t, float16x4_t>(bu2));
            let af3: float32x4_t =
                vcvt_f32_f16(std::mem::transmute::<uint16x4_t, float16x4_t>(au3));
            let bf3: float32x4_t =
                vcvt_f32_f16(std::mem::transmute::<uint16x4_t, float16x4_t>(bu3));

            acc0 = vmlaq_f32(acc0, af0, bf0);
            acc1 = vmlaq_f32(acc1, af1, bf1);
            acc2 = vmlaq_f32(acc2, af2, bf2);
            acc3 = vmlaq_f32(acc3, af3, bf3);
        }
        i += 16;
    }

    // 8-wide tail
    while i + 8 <= n {
        unsafe {
            let ap = a.as_ptr().add(i) as *const u16;
            let bp = b.as_ptr().add(i) as *const u16;
            let au0: uint16x4_t = vld1_u16(ap);
            let bu0: uint16x4_t = vld1_u16(bp);
            let au1: uint16x4_t = vld1_u16(ap.add(4));
            let bu1: uint16x4_t = vld1_u16(bp.add(4));
            let af0: float32x4_t =
                vcvt_f32_f16(std::mem::transmute::<uint16x4_t, float16x4_t>(au0));
            let bf0: float32x4_t =
                vcvt_f32_f16(std::mem::transmute::<uint16x4_t, float16x4_t>(bu0));
            let af1: float32x4_t =
                vcvt_f32_f16(std::mem::transmute::<uint16x4_t, float16x4_t>(au1));
            let bf1: float32x4_t =
                vcvt_f32_f16(std::mem::transmute::<uint16x4_t, float16x4_t>(bu1));
            acc0 = vmlaq_f32(acc0, af0, bf0);
            acc1 = vmlaq_f32(acc1, af1, bf1);
        }
        i += 8;
    }

    // merge accumulators
    let acc01 = unsafe { vaddq_f32(acc0, acc1) };
    let acc23 = unsafe { vaddq_f32(acc2, acc3) };
    let acc = unsafe { vaddq_f32(acc01, acc23) };
    let mut result: f32 = unsafe { vaddvq_f32(acc) };

    // scalar tail
    while i < n {
        unsafe {
            result += a.get_unchecked(i).to_f32() * b.get_unchecked(i).to_f32();
        }
        i += 1;
    }
    result
}

/// Public dispatch: NEON when feature=neon + aarch64, else scalar.
#[inline]
pub fn dot_f16(a: &[f16], b: &[f16]) -> f32 {
    #[cfg(all(target_arch = "aarch64", feature = "neon"))]
    {
        dot_f16_neon(a, b)
    }
    #[cfg(not(all(target_arch = "aarch64", feature = "neon")))]
    {
        dot_f16_scalar(a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use half::f16;

    fn v(xs: &[f32]) -> Vec<f16> {
        xs.iter().map(|&x| f16::from_f32(x)).collect()
    }

    #[test]
    fn correctness_small() {
        let a = v(&[1.0, 2.0, 3.0, 4.0]);
        let b = v(&[4.0, 3.0, 2.0, 1.0]);
        let got = dot_f16(&a, &b);
        assert!((got - 20.0f32).abs() < 0.1, "got={got}");
    }

    #[test]
    fn scalar_vs_neon_dim128() {
        let a: Vec<f16> = (0..128)
            .map(|i| f16::from_f32((i % 10) as f32 * 0.1))
            .collect();
        let b: Vec<f16> = (0..128)
            .map(|i| f16::from_f32(((127 - i) % 10) as f32 * 0.1))
            .collect();
        let s = dot_f16_scalar(&a, &b);
        let n = dot_f16(&a, &b);
        assert!((s - n).abs() < 0.5, "scalar={s} neon={n}");
    }

    #[test]
    fn scalar_vs_neon_dim256() {
        let a: Vec<f16> = (0..256)
            .map(|i| f16::from_f32((i % 20) as f32 * 0.05 - 0.5))
            .collect();
        let b: Vec<f16> = (0..256)
            .map(|i| f16::from_f32(((i * 3) % 20) as f32 * 0.05 - 0.5))
            .collect();
        let s = dot_f16_scalar(&a, &b);
        let n = dot_f16(&a, &b);
        assert!((s - n).abs() < 1.0, "scalar={s} neon={n}");
    }

    #[test]
    fn non_multiple_of_8() {
        let a: Vec<f16> = (0..37).map(|i| f16::from_f32(i as f32 * 0.1)).collect();
        let b: Vec<f16> = (0..37)
            .map(|i| f16::from_f32((37 - i) as f32 * 0.1))
            .collect();
        let s = dot_f16_scalar(&a, &b);
        let n = dot_f16(&a, &b);
        assert!((s - n).abs() < 0.5, "scalar={s} neon={n}");
    }
}
