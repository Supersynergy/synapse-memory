use super::neon;
/// High-level wrappers over column slices from decoded mmap pages.
use crate::store::page::{Bar, Page};

/// Aggregate kind for routed SimdAgg path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggKind {
    Mean,
    Vwap,
    Min,
    Max,
    Sum,
}

/// Scalar aggregate result — no bar materialization.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AggResult {
    pub kind: AggKind,
    pub value: f32,
    pub n_bars: usize,
}

/// Compute aggregate directly over page column slices — zero alloc, no Bar materialization.
pub fn agg_pages(pages: &[Page], kind: AggKind) -> AggResult {
    let n_bars: usize = pages.iter().map(|p| p.len()).sum();
    if n_bars == 0 {
        return AggResult {
            kind,
            value: 0.0,
            n_bars: 0,
        };
    }
    let value = match kind {
        AggKind::Mean => {
            let (mut sum, mut count) = (0.0f32, 0usize);
            for p in pages {
                sum += mean_close_slice(p.closes()) * p.len() as f32;
                count += p.len();
            }
            if count == 0 { 0.0 } else { sum / count as f32 }
        }
        AggKind::Vwap => {
            let (mut pv, mut vol) = (0.0f32, 0.0f32);
            for p in pages {
                // accumulate pv + vol across pages
                let third = 1.0 / 3.0;
                for i in 0..p.len() {
                    let tp = (p.highs()[i] + p.lows()[i] + p.closes()[i]) * third;
                    pv += tp * p.volumes()[i];
                    vol += p.volumes()[i];
                }
            }
            if vol < 1e-12 { 0.0 } else { pv / vol }
        }
        AggKind::Min => pages
            .iter()
            .flat_map(|p| p.closes().iter().copied())
            .fold(f32::MAX, f32::min),
        AggKind::Max => pages
            .iter()
            .flat_map(|p| p.closes().iter().copied())
            .fold(f32::MIN, f32::max),
        AggKind::Sum => pages.iter().flat_map(|p| p.closes().iter().copied()).sum(),
    };
    AggResult {
        kind,
        value,
        n_bars,
    }
}

// Byte offset of `close` field inside Bar: i64(8) + open(4) + high(4) + low(4) = 20
const CLOSE_OFFSET: usize = 20;

/// Mean close — strided gather over Bar slice, no alloc.
#[inline]
pub fn mean_close(bars: &[Bar]) -> f32 {
    neon::mean_strided_f32(bars, CLOSE_OFFSET)
}

/// Mean close from pre-extracted slice — zero alloc, direct SIMD.
#[inline]
pub fn mean_close_slice(closes: &[f32]) -> f32 {
    neon::mean_f32(closes)
}

/// Volume-weighted average price (VWAP) from Bar slice.
#[inline]
pub fn vwap(bars: &[Bar]) -> f32 {
    let mut vol_sum = 0.0f32;
    let mut pv_sum = 0.0f32;
    for b in bars {
        let typical = (b.high + b.low + b.close) / 3.0;
        pv_sum += typical * b.volume;
        vol_sum += b.volume;
    }
    if vol_sum < 1e-12 {
        0.0
    } else {
        pv_sum / vol_sum
    }
}

/// VWAP from pre-extracted column slices — SIMD vectorized.
#[inline]
pub fn vwap_slices(high: &[f32], low: &[f32], close: &[f32], volume: &[f32]) -> f32 {
    use wide::f32x8;
    let n = high.len().min(low.len()).min(close.len()).min(volume.len());
    let third = f32x8::splat(1.0 / 3.0);
    let mut pv_acc = f32x8::ZERO;
    let mut vol_acc = f32x8::ZERO;
    let full = n / 8;
    for i in 0..full {
        let b = i * 8;
        let h = f32x8::from([
            high[b],
            high[b + 1],
            high[b + 2],
            high[b + 3],
            high[b + 4],
            high[b + 5],
            high[b + 6],
            high[b + 7],
        ]);
        let l = f32x8::from([
            low[b],
            low[b + 1],
            low[b + 2],
            low[b + 3],
            low[b + 4],
            low[b + 5],
            low[b + 6],
            low[b + 7],
        ]);
        let c = f32x8::from([
            close[b],
            close[b + 1],
            close[b + 2],
            close[b + 3],
            close[b + 4],
            close[b + 5],
            close[b + 6],
            close[b + 7],
        ]);
        let v = f32x8::from([
            volume[b],
            volume[b + 1],
            volume[b + 2],
            volume[b + 3],
            volume[b + 4],
            volume[b + 5],
            volume[b + 6],
            volume[b + 7],
        ]);
        let tp = (h + l + c) * third;
        pv_acc += tp * v;
        vol_acc += v;
    }
    let arr_pv: [f32; 8] = pv_acc.into();
    let arr_vol: [f32; 8] = vol_acc.into();
    let mut pv_sum: f32 = arr_pv.iter().sum();
    let mut vol_sum: f32 = arr_vol.iter().sum();
    for i in full * 8..n {
        let typical = (high[i] + low[i] + close[i]) / 3.0;
        pv_sum += typical * volume[i];
        vol_sum += volume[i];
    }
    if vol_sum < 1e-12 {
        0.0
    } else {
        pv_sum / vol_sum
    }
}

/// mean(close) from SoA Page — zero alloc.
#[inline]
pub fn mean_close_page(page: &Page) -> f32 {
    mean_close_slice(page.closes())
}

/// VWAP from SoA Page — zero alloc.
#[inline]
pub fn vwap_page(page: &Page) -> f32 {
    vwap_slices(page.highs(), page.lows(), page.closes(), page.volumes())
}

/// Pearson correlation from SoA Pages — zero alloc.
#[inline]
pub fn pearson_correlation_pages(a: &Page, b: &Page) -> f32 {
    pearson_correlation_slices(a.closes(), b.closes())
}

/// Bar-over-bar log returns, rolling `window` bars.
pub fn rolling_returns(bars: &[Bar], window: usize) -> Vec<f32> {
    let n = bars.len();
    if n <= window || window == 0 {
        return vec![];
    }
    (window..n)
        .map(|i| {
            let prev = bars[i - window].close;
            if prev.abs() < 1e-12 {
                0.0
            } else {
                (bars[i].close / prev).ln()
            }
        })
        .collect()
}

/// Log returns from pre-extracted close slice.
pub fn rolling_returns_slice(closes: &[f32], window: usize) -> Vec<f32> {
    let n = closes.len();
    if n <= window || window == 0 {
        return vec![];
    }
    (window..n)
        .map(|i| {
            let prev = closes[i - window];
            if prev.abs() < 1e-12 {
                0.0
            } else {
                (closes[i] / prev).ln()
            }
        })
        .collect()
}

/// Pearson correlation between close prices of two Bar slices.
pub fn pearson_correlation(a: &[Bar], b: &[Bar]) -> f32 {
    let len = a.len().min(b.len());
    if len < 2 {
        return 0.0;
    }
    let ac: Vec<f32> = a[..len].iter().map(|b| b.close).collect();
    let bc: Vec<f32> = b[..len].iter().map(|b| b.close).collect();
    neon::correlation_f32(&ac, &bc)
}

/// Pearson correlation directly from pre-extracted slices — no collect.
#[inline]
pub fn pearson_correlation_slices(a: &[f32], b: &[f32]) -> f32 {
    neon::correlation_f32(a, b)
}

/// Rolling mean of close prices over `window` bars.
pub fn rolling_mean_close(bars: &[Bar], window: usize) -> Vec<f32> {
    let closes: Vec<f32> = bars.iter().map(|b| b.close).collect();
    neon::rolling_mean_f32(&closes, window)
}

/// Rolling mean from pre-extracted close slice — no collect.
#[inline]
pub fn rolling_mean_slice(closes: &[f32], window: usize) -> Vec<f32> {
    neon::rolling_mean_f32(closes, window)
}

/// Rolling std of close prices.
pub fn rolling_std_close(bars: &[Bar], window: usize) -> Vec<f32> {
    let closes: Vec<f32> = bars.iter().map(|b| b.close).collect();
    neon::rolling_std_f32(&closes, window)
}

/// Rolling std from pre-extracted close slice.
#[inline]
pub fn rolling_std_slice(closes: &[f32], window: usize) -> Vec<f32> {
    neon::rolling_std_f32(closes, window)
}

/// EWMA of close prices.
pub fn ewma_close(bars: &[Bar], alpha: f32) -> Vec<f32> {
    let closes: Vec<f32> = bars.iter().map(|b| b.close).collect();
    neon::ewma_f32(&closes, alpha)
}

/// EWMA from pre-extracted close slice — no collect.
#[inline]
pub fn ewma_slice(closes: &[f32], alpha: f32) -> Vec<f32> {
    neon::ewma_f32(closes, alpha)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::page::Bar;

    fn make_bars(n: usize) -> Vec<Bar> {
        (0..n)
            .map(|i| Bar {
                ts: i as i64 * 900,
                open: 100.0,
                high: 101.0,
                low: 99.0,
                close: 100.0 + i as f32 * 0.1,
                volume: 1000.0,
            })
            .collect()
    }

    #[test]
    fn mean_close_trivial() {
        let bars = make_bars(10);
        let m = mean_close(&bars);
        assert!(m > 99.0 && m < 102.0);
    }

    #[test]
    fn mean_close_slice_matches() {
        let bars = make_bars(64);
        let closes: Vec<f32> = bars.iter().map(|b| b.close).collect();
        let m1 = mean_close(&bars);
        let m2 = mean_close_slice(&closes);
        assert!((m1 - m2).abs() < 1e-4, "slice mismatch: {m1} vs {m2}");
    }

    #[test]
    fn vwap_reasonable() {
        let bars = make_bars(20);
        let v = vwap(&bars);
        assert!(v > 99.0 && v < 103.0);
    }

    #[test]
    fn vwap_slices_matches() {
        let bars = make_bars(64);
        let high: Vec<f32> = bars.iter().map(|b| b.high).collect();
        let low: Vec<f32> = bars.iter().map(|b| b.low).collect();
        let close: Vec<f32> = bars.iter().map(|b| b.close).collect();
        let volume: Vec<f32> = bars.iter().map(|b| b.volume).collect();
        let v1 = vwap(&bars);
        let v2 = vwap_slices(&high, &low, &close, &volume);
        assert!((v1 - v2).abs() < 1e-4, "vwap mismatch: {v1} vs {v2}");
    }

    #[test]
    fn rolling_returns_len() {
        let bars = make_bars(100);
        let r = rolling_returns(&bars, 20);
        assert_eq!(r.len(), 80);
    }

    #[test]
    fn pearson_self_is_one() {
        let bars = make_bars(64);
        let r = pearson_correlation(&bars, &bars);
        assert!((r - 1.0).abs() < 1e-4);
    }

    #[test]
    fn pearson_slices_self_is_one() {
        let bars = make_bars(64);
        let closes: Vec<f32> = bars.iter().map(|b| b.close).collect();
        let r = pearson_correlation_slices(&closes, &closes);
        assert!((r - 1.0).abs() < 1e-4);
    }

    #[test]
    fn ewma_slice_matches() {
        let bars = make_bars(100);
        let closes: Vec<f32> = bars.iter().map(|b| b.close).collect();
        let alpha = 2.0 / 21.0;
        let v1 = ewma_close(&bars, alpha);
        let v2 = ewma_slice(&closes, alpha);
        assert_eq!(v1.len(), v2.len());
        for (a, b) in v1.iter().zip(v2.iter()) {
            assert!((a - b).abs() < 1e-5);
        }
    }

    #[test]
    fn rolling_mean_slice_matches() {
        let bars = make_bars(100);
        let closes: Vec<f32> = bars.iter().map(|b| b.close).collect();
        let v1 = rolling_mean_close(&bars, 20);
        let v2 = rolling_mean_slice(&closes, 20);
        assert_eq!(v1.len(), v2.len());
        for (a, b) in v1.iter().zip(v2.iter()) {
            assert!((a - b).abs() < 1e-5);
        }
    }
}
