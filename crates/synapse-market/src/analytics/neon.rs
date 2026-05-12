/// SIMD-accelerated analytic kernels using `wide` (portable NEON/AVX f32x8).
use wide::f32x8;

// ── helpers ───────────────────────────────────────────────────────────────────

#[inline(always)]
fn reduce_sum(acc: f32x8) -> f32 {
    let arr: [f32; 8] = acc.into();
    arr.iter().sum()
}

#[inline(always)]
fn load8(xs: &[f32], i: usize) -> f32x8 {
    f32x8::from([xs[i], xs[i+1], xs[i+2], xs[i+3], xs[i+4], xs[i+5], xs[i+6], xs[i+7]])
}

// ── public kernels ────────────────────────────────────────────────────────────

pub fn sum_f32(xs: &[f32]) -> f32 {
    let n = xs.len();
    let full = n / 8;
    let mut acc = f32x8::ZERO;
    for i in 0..full {
        acc += load8(xs, i * 8);
    }
    let mut s = reduce_sum(acc);
    for i in full*8..n { s += xs[i]; }
    s
}

pub fn mean_f32(xs: &[f32]) -> f32 {
    if xs.is_empty() { return 0.0; }
    sum_f32(xs) / xs.len() as f32
}

pub fn max_f32(xs: &[f32]) -> f32 {
    if xs.is_empty() { return f32::NEG_INFINITY; }
    let n = xs.len();
    let full = n / 8;
    let mut acc = f32x8::splat(f32::NEG_INFINITY);
    for i in 0..full {
        acc = acc.max(load8(xs, i * 8));
    }
    let arr: [f32; 8] = acc.into();
    let mut m = arr.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    for i in full*8..n { m = m.max(xs[i]); }
    m
}

pub fn min_f32(xs: &[f32]) -> f32 {
    if xs.is_empty() { return f32::INFINITY; }
    let n = xs.len();
    let full = n / 8;
    let mut acc = f32x8::splat(f32::INFINITY);
    for i in 0..full {
        acc = acc.min(load8(xs, i * 8));
    }
    let arr: [f32; 8] = acc.into();
    let mut m = arr.iter().cloned().fold(f32::INFINITY, f32::min);
    for i in full*8..n { m = m.min(xs[i]); }
    m
}

pub fn dot_f32(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let full = n / 8;
    let mut acc = f32x8::ZERO;
    for i in 0..full {
        acc += load8(a, i*8) * load8(b, i*8);
    }
    let mut s = reduce_sum(acc);
    for i in full*8..n { s += a[i] * b[i]; }
    s
}

pub fn correlation_f32(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    if n < 2 { return 0.0; }
    let mean_a = mean_f32(&a[..n]);
    let mean_b = mean_f32(&b[..n]);
    // Single-pass SIMD: no alloc
    let ma = f32x8::splat(mean_a);
    let mb = f32x8::splat(mean_b);
    let mut num  = f32x8::ZERO;
    let mut da2  = f32x8::ZERO;
    let mut db2  = f32x8::ZERO;
    let full = n / 8;
    for i in 0..full {
        let ai = load8(a, i * 8) - ma;
        let bi = load8(b, i * 8) - mb;
        num += ai * bi;
        da2 += ai * ai;
        db2 += bi * bi;
    }
    let mut snum = reduce_sum(num);
    let mut sda2 = reduce_sum(da2);
    let mut sdb2 = reduce_sum(db2);
    for i in full*8..n {
        let ai = a[i] - mean_a;
        let bi = b[i] - mean_b;
        snum += ai * bi;
        sda2 += ai * ai;
        sdb2 += bi * bi;
    }
    let denom = (sda2 * sdb2).sqrt();
    if denom < 1e-12 { 0.0 } else { snum / denom }
}

pub fn ewma_f32(xs: &[f32], alpha: f32) -> Vec<f32> {
    if xs.is_empty() { return vec![]; }
    let mut out = Vec::with_capacity(xs.len());
    let mut s = xs[0];
    out.push(s);
    for &x in &xs[1..] {
        s = alpha * x + (1.0 - alpha) * s;
        out.push(s);
    }
    out
}

pub fn rolling_mean_f32(xs: &[f32], window: usize) -> Vec<f32> {
    let n = xs.len();
    if n < window || window == 0 { return vec![]; }
    let mut out = Vec::with_capacity(n - window + 1);
    let inv = 1.0 / window as f32;
    // bootstrap first window
    let mut sum: f32 = xs[..window].iter().sum();
    out.push(sum * inv);
    for i in window..n {
        sum += xs[i] - xs[i - window];
        out.push(sum * inv);
    }
    out
}

pub fn rolling_std_f32(xs: &[f32], window: usize) -> Vec<f32> {
    let n = xs.len();
    if n < window || window == 0 { return vec![]; }
    let mut out = Vec::with_capacity(n - window + 1);
    let inv = 1.0 / window as f32;
    for i in 0..=(n - window) {
        let slice = &xs[i..i + window];
        let mean = slice.iter().sum::<f32>() * inv;
        let var = slice.iter().map(|&x| (x - mean) * (x - mean)).sum::<f32>() * inv;
        out.push(var.sqrt());
    }
    out
}

/// Mean of a strided field inside a struct. `stride` = size_of::<T>().
/// `byte_offset` = offset of the f32 field inside T.
/// SAFETY: caller guarantees T has f32 at byte_offset.
pub fn mean_strided_f32<T>(slice: &[T], byte_offset: usize) -> f32 {
    if slice.is_empty() { return 0.0; }
    let n = slice.len();
    let stride = std::mem::size_of::<T>();
    let base = slice.as_ptr() as *const u8;
    let mut sum = 0.0f32;
    for i in 0..n {
        let ptr = unsafe { base.add(i * stride + byte_offset) as *const f32 };
        sum += unsafe { ptr.read_unaligned() };
    }
    sum / n as f32
}

/// Scalar (non-SIMD) fallback implementations — used as baseline in benches.
pub mod scalar {
    pub fn rolling_mean_f32(xs: &[f32], window: usize) -> Vec<f32> {
        let n = xs.len();
        if n < window || window == 0 { return vec![]; }
        let inv = 1.0 / window as f32;
        let mut out = Vec::with_capacity(n - window + 1);
        let mut sum: f32 = xs[..window].iter().sum();
        out.push(sum * inv);
        for i in window..n {
            sum += xs[i] - xs[i - window];
            out.push(sum * inv);
        }
        out
    }

    pub fn ewma_f32(xs: &[f32], alpha: f32) -> Vec<f32> {
        if xs.is_empty() { return vec![]; }
        let mut out = Vec::with_capacity(xs.len());
        let mut s = xs[0];
        out.push(s);
        for &x in &xs[1..] {
            s = alpha * x + (1.0 - alpha) * s;
            out.push(s);
        }
        out
    }

    pub fn correlation_f32(a: &[f32], b: &[f32]) -> f32 {
        let n = a.len().min(b.len());
        if n < 2 { return 0.0; }
        let mean_a = a[..n].iter().sum::<f32>() / n as f32;
        let mean_b = b[..n].iter().sum::<f32>() / n as f32;
        let mut num = 0.0f32;
        let mut da2 = 0.0f32;
        let mut db2 = 0.0f32;
        for i in 0..n {
            let da = a[i] - mean_a;
            let db = b[i] - mean_b;
            num += da * db;
            da2 += da * da;
            db2 += db * db;
        }
        let denom = (da2 * db2).sqrt();
        if denom < 1e-12 { 0.0 } else { num / denom }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mean_trivial() {
        let xs = vec![1.0f32, 2.0, 3.0, 4.0];
        assert!((mean_f32(&xs) - 2.5).abs() < 1e-6);
    }

    #[test]
    fn correlation_self() {
        let xs: Vec<f32> = (0..64).map(|i| i as f32).collect();
        assert!((correlation_f32(&xs, &xs) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn rolling_mean_window() {
        let xs = vec![1.0f32, 2.0, 3.0, 4.0, 5.0];
        let r = rolling_mean_f32(&xs, 3);
        assert_eq!(r.len(), 3);
        assert!((r[0] - 2.0).abs() < 1e-6);
        assert!((r[1] - 3.0).abs() < 1e-6);
        assert!((r[2] - 4.0).abs() < 1e-6);
    }

    #[test]
    fn ewma_basic() {
        let xs = vec![1.0f32; 10];
        let r = ewma_f32(&xs, 0.5);
        assert_eq!(r.len(), 10);
        assert!((r[9] - 1.0).abs() < 1e-4);
    }
}
