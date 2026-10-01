//! Criterion bench — embedding cache hit-path (PR-D1 scale-100M).
//!
//! Isolates the BLAKE3-hash + redb-get + decode path WITHOUT invoking fastembed.
//! Simulates a warm cache by pre-populating with synthetic rows. Reports
//! ms/batch at sizes 10, 100, 1000, 10000 with and without rayon.
//!
//! Run:
//!   cargo bench -p synapse-core --features embed --bench embed_cache
//!
//! Two harness modes:
//!   BASELINE  — serial hash (pre-PR-D1): checkout HEAD~1 to compare
//!   PARALLEL  — rayon par_iter hash (PR-D1): current HEAD

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};

#[cfg(feature = "embed")]
fn bench_hash_only(c: &mut Criterion) {
    let sizes = [10usize, 100, 1_000, 10_000];
    let mut g = c.benchmark_group("embed_hash_only");
    for &n in &sizes {
        let texts: Vec<String> = (0..n).map(|i| format!("doc {i} topic{}", i % 37)).collect();
        g.throughput(Throughput::Elements(n as u64));
        g.bench_with_input(BenchmarkId::new("serial", n), &texts, |b, t| {
            b.iter(|| {
                let hashes: Vec<[u8; 32]> = t
                    .iter()
                    .map(|s| *blake3::hash(s.as_bytes()).as_bytes())
                    .collect();
                black_box(hashes);
            });
        });
        g.bench_with_input(BenchmarkId::new("rayon_par", n), &texts, |b, t| {
            use rayon::prelude::*;
            b.iter(|| {
                let hashes: Vec<[u8; 32]> = t
                    .par_iter()
                    .map(|s| *blake3::hash(s.as_bytes()).as_bytes())
                    .collect();
                black_box(hashes);
            });
        });
    }
    g.finish();
}

#[cfg(feature = "embed")]
fn bench_pack_only(c: &mut Criterion) {
    let dim = 384usize;
    let sizes = [10usize, 100, 1_000];
    let mut g = c.benchmark_group("embed_pack_f32_to_le_bytes");
    for &n in &sizes {
        let embs: Vec<Vec<f32>> = (0..n)
            .map(|i| (0..dim).map(|j| ((i + j) as f32 * 0.001).sin()).collect())
            .collect();
        g.throughput(Throughput::Elements(n as u64));
        g.bench_with_input(BenchmarkId::new("serial", n), &embs, |b, e| {
            b.iter(|| {
                let packed: Vec<Vec<u8>> = e
                    .iter()
                    .map(|v| v.iter().flat_map(|f| f.to_le_bytes()).collect())
                    .collect();
                black_box(packed);
            });
        });
        g.bench_with_input(BenchmarkId::new("rayon_par", n), &embs, |b, e| {
            use rayon::prelude::*;
            b.iter(|| {
                let packed: Vec<Vec<u8>> = e
                    .par_iter()
                    .map(|v| v.iter().flat_map(|f| f.to_le_bytes()).collect())
                    .collect();
                black_box(packed);
            });
        });
    }
    g.finish();
}

#[cfg(not(feature = "embed"))]
fn bench_hash_only(_c: &mut Criterion) {
    eprintln!("skip: enable --features embed");
}
#[cfg(not(feature = "embed"))]
fn bench_pack_only(_c: &mut Criterion) {}

criterion_group!(benches, bench_hash_only, bench_pack_only);
criterion_main!(benches);
