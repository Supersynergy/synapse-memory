/// Bench: hot (raw mmap) vs cold (zstd-19) — size ratio and range-scan cost.
use criterion::{criterion_group, criterion_main, Criterion};
use synapse_market::store::compact::{ColdTier, cold_stats};
use synapse_market::store::page::{encode_page, decode_page, Bar, PAGE_SIZE};
use tempfile::TempDir;

fn make_bars(n: usize, base_ts: i64) -> Vec<Bar> {
    (0..n).map(|i| Bar {
        ts: base_ts + i as i64 * 900,
        open: 100.0 + i as f32 * 0.1,
        high: 101.0 + i as f32 * 0.1,
        low: 99.0 + i as f32 * 0.1,
        close: 100.5 + i as f32 * 0.1,
        volume: 1000.0 + i as f32,
    }).collect()
}

fn bench_compression_ratio(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let csm = dir.path().join("bench.csm");

    // 1000 pages × PAGE_SIZE = ~64MB hot
    let n_pages = 1000;
    let base = 1_700_000_000i64;
    let raw_pages: Vec<Vec<u8>> = (0..n_pages).map(|p| {
        let bars = make_bars(100, base + p as i64 * 100 * 900);
        encode_page(&bars)
    }).collect();

    // Create cold file (one-time setup, not benched)
    ColdTier::create(&csm, &raw_pages).unwrap();

    let stats = cold_stats(&csm).unwrap();
    println!(
        "\n[compression_ratio] hot={:.1}MB cold={:.1}MB ratio={:.2}× savings={:.1}%",
        stats.hot_bytes as f64 / 1_048_576.0,
        stats.total_compressed_bytes as f64 / 1_048_576.0,
        stats.compression_ratio(),
        (1.0 - 1.0 / stats.compression_ratio()) * 100.0,
    );

    // Bench hot scan: decode all pages from in-memory Vec<u8>
    c.bench_function("hot_scan_1000pages", |b| {
        b.iter(|| {
            let mut total = 0usize;
            for raw in &raw_pages {
                let (_, bars) = decode_page(std::hint::black_box(raw));
                total += bars.len();
            }
            total
        });
    });

    // Bench cold scan: decompress + decode 1000 pages (cold, no LRU warmup)
    c.bench_function("cold_scan_1000pages", |b| {
        b.iter(|| {
            let mut cold = ColdTier::open(std::hint::black_box(&csm)).unwrap();
            let mut total = 0usize;
            for i in 0..n_pages as u32 {
                let bars = cold.read_page(i).unwrap();
                total += bars.len();
            }
            total
        });
    });

    // Bench cold scan with warm LRU (all 1000 pages pre-loaded into a new instance)
    // Note: LRU capacity = 50, so only tail 50 pages stay cached.
    c.bench_function("cold_scan_warm_lru_50", |b| {
        let mut cold = ColdTier::open(&csm).unwrap();
        // Warm last 50 pages
        for i in (n_pages - 50) as u32..n_pages as u32 {
            cold.read_page(i).unwrap();
        }
        b.iter(|| {
            let mut total = 0usize;
            for i in (n_pages - 50) as u32..n_pages as u32 {
                let bars = cold.read_page(std::hint::black_box(i)).unwrap();
                total += bars.len();
            }
            total
        });
    });
}

criterion_group!(benches, bench_compression_ratio);
criterion_main!(benches);
