use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use synapse_market::series::Series;
use synapse_market::store::page::Bar;
use tempfile::TempDir;

fn make_bars(n: usize, base_ts: i64) -> Vec<Bar> {
    (0..n)
        .map(|i| Bar {
            ts: base_ts + i as i64 * 900,
            open: 100.0,
            high: 101.0,
            low: 99.0,
            close: 100.5,
            volume: 1000.0,
        })
        .collect()
}

fn bench_bloom_neglookup(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();

    // Load 220 tickers, each with ~1000 bars (15-min bars, ~10 days)
    let n_tickers = 220usize;
    let bars_per_ticker = 1000usize;
    let base_ts = 1_700_000_000i64;
    let series_duration = bars_per_ticker as i64 * 900;

    let paths: Vec<_> = (0..n_tickers)
        .map(|i| dir.path().join(format!("ticker_{i}.smx")))
        .collect();

    for path in &paths {
        let mut s = Series::open(path).unwrap();
        s.append(&make_bars(bars_per_ticker, base_ts)).unwrap();
        s.close().unwrap();
    }

    // Re-open for queries
    let mut series: Vec<Series> = paths.iter().map(|p| Series::open(p).unwrap()).collect();

    // 1000 queries: 500 ts in-range (existing), 500 out-of-range (negative)
    let in_range_queries: Vec<_> = (0..500usize)
        .map(|i| {
            let mid = base_ts + (i as i64 % bars_per_ticker as i64) * 900;
            mid..mid + 900
        })
        .collect();
    let out_range_queries: Vec<_> = (0..500usize)
        .map(|i| {
            let far = base_ts + series_duration + (i as i64 + 1) * 900 * 1000;
            far..far + 900
        })
        .collect();

    let mut group = c.benchmark_group("bloom_neglookup");

    group.bench_function("with_bloom_negative", |b| {
        b.iter(|| {
            let mut count = 0usize;
            for q in &out_range_queries {
                let s = &mut series[count % n_tickers];
                count += 1;
                let r = s.range_filter(q.clone()).unwrap();
                criterion::black_box(r);
            }
        })
    });

    group.bench_function("without_bloom_negative", |b| {
        b.iter(|| {
            let mut count = 0usize;
            for q in &out_range_queries {
                let s = &mut series[count % n_tickers];
                count += 1;
                // range() has no bloom guard
                let r = s.range(q.clone()).unwrap();
                criterion::black_box(r);
            }
        })
    });

    group.bench_function("with_bloom_positive", |b| {
        b.iter(|| {
            let mut count = 0usize;
            for q in &in_range_queries {
                let s = &mut series[count % n_tickers];
                count += 1;
                let r = s.range_filter(q.clone()).unwrap();
                criterion::black_box(r);
            }
        })
    });

    group.finish();
}

criterion_group!(benches, bench_bloom_neglookup);
criterion_main!(benches);
