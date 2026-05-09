//! Bench: AutoloadCache lookup vs hypothetical SQL roundtrip.
//!
//! Real WP loads `wp_options WHERE autoload='yes'` (200-2000 rows) on every
//! pageload. Even on libsql local that's ~1ms. AutoloadCache is HashMap<String, Vec<u8>>
//! → expected sub-µs lookup.
//!
//! This bench measures the absolute cache lookup cost. Compare to libsql
//! exec ~1ms (3 orders of magnitude).

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use synapse_wp::AutoloadCache;

fn populate_cache(n: usize) -> AutoloadCache {
    let c = AutoloadCache::new();
    let rows = (0..n).map(|i| (format!("opt_{i}"), format!("value_{i}").into_bytes()));
    c.load_all(rows);
    c
}

fn bench_get(c: &mut Criterion) {
    for n in [200usize, 1000, 2000] {
        let cache = populate_cache(n);
        c.bench_function(&format!("autoload_get_n{n}"), |b| {
            b.iter(|| {
                let v = cache.get(black_box("opt_42"));
                black_box(v);
            });
        });
    }
}

fn bench_full_pageload_simulation(c: &mut Criterion) {
    // Simulate WP pageload: 30 distinct option lookups
    let cache = populate_cache(2000);
    let keys: Vec<String> = (0..30).map(|i| format!("opt_{i}")).collect();
    c.bench_function("wp_pageload_30_options_n2000", |b| {
        b.iter(|| {
            for k in &keys {
                let _ = black_box(cache.get(k));
            }
        });
    });
}

criterion_group!(benches, bench_get, bench_full_pageload_simulation);
criterion_main!(benches);
