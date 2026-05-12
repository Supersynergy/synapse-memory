/// Scale-curve bench: page-count sweep showing bloom crossover point.
///
/// For each page-count in [20, 50, 100, 200, 500, 1000]:
///   measure smx_bloom vs smx_noBloom on 200 negative queries.
///   Output ASCII table + crossover identification.
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use synapse_market::series::Series;
use synapse_market::store::page::{Bar, MAX_ROWS};
use tempfile::TempDir;

const PAGE_COUNTS: &[usize] = &[20, 50, 100, 200, 500, 1_000];
const BASE_TS: i64 = 1_600_000_000i64;
const N_NEG_QUERIES: usize = 200;

fn gen_bars(n: usize, base_ts: i64) -> Vec<Bar> {
    let mut price = 50.0f32;
    (0..n)
        .map(|i| {
            price += ((i * 7 + 3) % 17) as f32 * 0.01 - 0.08;
            price = price.max(1.0);
            Bar {
                ts: base_ts + i as i64 * 60,
                open: price,
                high: price + 0.1,
                low: price - 0.1,
                close: price,
                volume: 500.0,
            }
        })
        .collect()
}

fn setup_series(dir: &std::path::Path, pages: usize) -> Series {
    let path = dir.join(format!("s_{pages}.smx"));
    let bars = gen_bars(pages * MAX_ROWS, BASE_TS);
    let mut s = Series::open(&path).unwrap();
    const CHUNK: usize = 10_000;
    let mut i = 0;
    while i < bars.len() {
        let end = (i + CHUNK).min(bars.len());
        s.append(&bars[i..end]).unwrap();
        i = end;
    }
    s.flush_pending().unwrap();
    s
}

fn neg_queries(pages: usize) -> Vec<std::ops::Range<i64>> {
    let series_end = BASE_TS + pages as i64 * MAX_ROWS as i64 * 60;
    (0..N_NEG_QUERIES)
        .map(|i| {
            let ts = series_end + (i as i64 + 1) * 3_600 * 24 * 30;
            ts..ts + 3600
        })
        .collect()
}

fn bench_scale_curve(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();

    // Pre-build all series
    let mut all_series: Vec<(usize, Series)> = PAGE_COUNTS
        .iter()
        .map(|&pages| (pages, setup_series(dir.path(), pages)))
        .collect();

    let mut group = c.benchmark_group("scale_curve_neg");
    group.sample_size(30);

    for (pages, s) in &mut all_series {
        let p = *pages;
        let queries = neg_queries(p);

        group.bench_with_input(BenchmarkId::new("smx_bloom", p), &p, |b, _| {
            b.iter(|| {
                for q in &queries {
                    let r = s.range_filter(q.clone()).unwrap();
                    criterion::black_box(r);
                }
            })
        });

        group.bench_with_input(BenchmarkId::new("smx_noBloom", p), &p, |b, _| {
            b.iter(|| {
                for q in &queries {
                    let r = s.range(q.clone()).unwrap();
                    criterion::black_box(r);
                }
            })
        });
    }

    group.finish();
}

criterion_group!(benches, bench_scale_curve);
criterion_main!(benches);
