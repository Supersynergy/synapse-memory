//! NEON FMLAL2 f16 dot kernel — M4 hot-path target.
//! TODO: replace with hand-tuned aarch64 intrinsics (vfmlalq_high_f16).
//! Currently scalar fallback to compile clean.

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn dot_f16(a: &[u16], b: &[u16]) -> f32 {
    // Stub: bit-cast u16→f16 not stable, do f32 conversion fallback.
    // Replace with `vld1q_f16` + `vfmlalq_high_f16` once `feature(stdarch_aarch64_f16)` lands.
    let mut acc = 0f32;
    for i in 0..a.len() {
        let af = half_to_f32(a[i]);
        let bf = half_to_f32(b[i]);
        acc += af * bf;
    }
    acc
}

#[cfg(not(target_arch = "aarch64"))]
pub fn dot_f16(_a: &[u16], _b: &[u16]) -> f32 {
    0.0
}

#[inline]
fn half_to_f32(h: u16) -> f32 {
    let s = ((h >> 15) & 1) as u32;
    let e = ((h >> 10) & 0x1f) as u32;
    let m = (h & 0x3ff) as u32;
    let bits = if e == 0 {
        if m == 0 {
            s << 31
        } else {
            let mut e2 = 1;
            let mut m2 = m;
            while (m2 & 0x400) == 0 {
                m2 <<= 1;
                e2 -= 1;
            }
            (s << 31) | (((127 - 15 + e2) as u32) << 23) | ((m2 & 0x3ff) << 13)
        }
    } else if e == 0x1f {
        (s << 31) | (0xff << 23) | (m << 13)
    } else {
        (s << 31) | ((127 - 15 + e) << 23) | (m << 13)
    };
    f32::from_bits(bits)
}
