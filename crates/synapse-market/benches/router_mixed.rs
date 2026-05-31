use criterion::{Criterion, criterion_group, criterion_main};
/// router_mixed — mixed-workload: 50% range, 30% aggregate, 20% full-scan.
/// Static path materialises bars for all queries.
/// Routed path: range→MmapScanSkipped (coverage<40%), agg→SimdAgg (no materialization).
/// Target: routed ≥1.2× faster on mixed workload.
use tempfile::TempDir;

use synapse_market::analytics::AggKind;
use synapse_market::router::PlanCache;
use synapse_market::series::Series;
use synapse_market::store::page::Bar;

const BARS: usize = 28800; // 10 pages worth → page-skip + alloc savings are meaningful
const BASE_TS: i64 = 1_700_000_000;
const ITERS: usize = 1000;

fn make_bars() -> Vec<Bar> {
    (0..BARS)
        .map(|i| Bar {
            ts: BASE_TS + i as i64 * 900,
            open: 100.0,
            high: 101.0,
            low: 99.0,
            close: 100.5,
            volume: 1000.0,
        })
        .collect()
}

/// 50% short-range, 30% aggregate (mean over full series), 20% full-scan
fn query_kind(i: usize) -> u8 {
    let r = i % 10;
    if r < 5 {
        0
    }
    // range (short, 100 bars, coverage ~3%)
    else if r < 8 {
        1
    }
    // aggregate mean over FULL series (all pages)
    else {
        2
    } // full scan
}

fn query_range(i: usize) -> (i64, i64) {
    match query_kind(i) {
        0 => {
            let start = BASE_TS + (i as i64 % 500) * 900;
            (start, start + 100 * 900)
        }
        1 => {
            // aggregate over full series — max benefit from skipping Bar alloc
            (BASE_TS, BASE_TS + BARS as i64 * 900)
        }
        _ => (BASE_TS, BASE_TS + BARS as i64 * 900),
    }
}

fn mixed_workload_static(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("series.smx");
    let mut s = Series::open(&path).unwrap();
    s.append(&make_bars()).unwrap();
    s.flush_pending().unwrap();

    c.bench_function("mixed_static_plan", |b| {
        b.iter(|| {
            for i in 0..ITERS {
                let (start, end) = query_range(i);
                // Static: always materialise bars, even for aggregates
                let bars = s.range(start..end).unwrap();
                if query_kind(i) == 1 {
                    // simulate aggregate: compute mean manually
                    if !bars.is_empty() {
                        let _mean: f32 =
                            bars.iter().map(|b| b.close).sum::<f32>() / bars.len() as f32;
                    }
                }
            }
        })
    });
}

fn mixed_workload_routed(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("series.smx");
    let mut s = Series::open(&path).unwrap();
    s.append(&make_bars()).unwrap();
    s.flush_pending().unwrap();
    let mut cache = PlanCache::new(256);

    c.bench_function("mixed_routed_plan", |b| {
        b.iter(|| {
            for i in 0..ITERS {
                let (start, end) = query_range(i);
                match query_kind(i) {
                    1 => {
                        // SimdAgg path — no Bar materialisation
                        let _ = s
                            .aggregate_routed(start..end, AggKind::Mean, &mut cache)
                            .unwrap();
                    }
                    _ => {
                        // MmapScanSkipped or Full based on coverage
                        let _ = s.range_routed(start..end, &mut cache).unwrap();
                    }
                }
            }
        })
    });
}

criterion_group!(benches, mixed_workload_static, mixed_workload_routed);
criterion_main!(benches);
