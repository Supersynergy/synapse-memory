/// router_mixed — 1000 mixed-workload queries comparing static plan vs routed.
/// Workload: 60% short-range, 30% long-range, 10% filter.
/// Target: routed ~1.3-1.5× faster on mixed workload.
use tempfile::TempDir;
use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};

use synapse_market::store::page::Bar;
use synapse_market::series::Series;
use synapse_market::router::{Plan, QueryKey, QueryKind, PlanCache};

const BARS: usize = 2880;
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
                let _ = s.range(start..end).unwrap();
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
                let _ = s.range_routed(start..end, &mut cache).unwrap();
            }
        })
    });
}

fn query_range(i: usize) -> (i64, i64) {
    let r = i % 10;
    if r < 6 {
        // 60% short-range: 100 bars
        let start = BASE_TS + (i as i64 % 1000) * 900;
        (start, start + 100 * 900)
    } else if r < 9 {
        // 30% long-range: all bars
        (BASE_TS, BASE_TS + BARS as i64 * 900)
    } else {
        // 10% filter-like: mid-range 500 bars
        let start = BASE_TS + 500 * 900;
        (start, start + 500 * 900)
    }
}

criterion_group!(benches, mixed_workload_static, mixed_workload_routed);
criterion_main!(benches);
