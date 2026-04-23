//! Criterion benchmarks — Turbo search strategies head-to-head.
//!
//! Compares: sqlite-vec vs ndarray f32 vs SIMD f32 vs int8 quantized
//! at various corpus sizes (100, 1k, 5k docs).
//!
//! Run:
//!   cargo bench -p synapse-core --features turbo --bench turbo

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use synapsedb_core::turbo::matryoshka::MatryoshkaConfig;
use synapsedb_core::turbo::ndarray_search::NdArraySearch;
use synapsedb_core::turbo::simd;
use synapsedb_core::types::EMBED_DIM;
use synapsedb_core::{PutRequest, SearchMode, Store};

fn fake_emb(seed: u8) -> Vec<f32> {
    (0..EMBED_DIM)
        .map(|i| ((i as u8).wrapping_mul(seed) as f32) / 255.0)
        .collect()
}

fn build_store(n: usize) -> (tempfile::NamedTempFile, Store) {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let mut store = Store::open(tmp.path()).unwrap();
    for i in 1..=n {
        store
            .put(&PutRequest {
                title: Some(format!("doc-{i}")),
                text: format!("document number {i} about topic {}", (i as u8) % 5),
                embedding: Some(fake_emb(i as u8)),
                ..Default::default()
            })
            .unwrap();
    }
    (tmp, store)
}

fn bench_search_strategies(c: &mut Criterion) {
    let mut group = c.benchmark_group("search_strategy");
    group.sample_size(200);

    for n in [100, 1000, 5000] {
        let (tmp, store) = build_store(n);
        let nd_search = NdArraySearch::from_sqlite(tmp.path()).unwrap();
        let quantized = nd_search.to_quantized();
        let matryoshka = nd_search.to_matryoshka(MatryoshkaConfig::default());
        let binary = nd_search.to_binary(true);
        let query = fake_emb(42);
        let k = 10;

        group.bench_with_input(
            BenchmarkId::new("sqlite-vec", n),
            &n,
            |b, _| {
                b.iter(|| {
                    let _ = store.search(
                        black_box(""),
                        SearchMode::Vec,
                        Some(black_box(&query)),
                        black_box(k),
                    );
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("ndarray-f32", n),
            &n,
            |b, _| {
                b.iter(|| {
                    let _ = nd_search.search(black_box(&query), black_box(k));
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("simd-f32", n),
            &n,
            |b, _| {
                b.iter(|| {
                    let _ = nd_search.search_simd(black_box(&query), black_box(k));
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("quantized-i8", n),
            &n,
            |b, _| {
                b.iter(|| {
                    let _ = quantized.search(black_box(&query), black_box(k));
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("matryoshka-funnel", n),
            &n,
            |b, _| {
                b.iter(|| {
                    let _ = matryoshka.funnel_search(black_box(&query), black_box(k));
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("binary-twophase", n),
            &n,
            |b, _| {
                b.iter(|| {
                    let _ = binary.search_twophase(black_box(&query), black_box(k), 20);
                })
            },
        );
    }

    group.finish();
}

fn bench_simd_dot_product(c: &mut Criterion) {
    let mut group = c.benchmark_group("dot_product");
    group.sample_size(1000);

    let a: Vec<f32> = (0..EMBED_DIM).map(|i| (i as f32) / EMBED_DIM as f32).collect();
    let b: Vec<f32> = (0..EMBED_DIM).map(|i| 1.0 - (i as f32) / EMBED_DIM as f32).collect();

    group.bench_function("f32-scalar-384d", |bench| {
        bench.iter(|| simd::dot_f32_scalar(black_box(&a), black_box(&b)))
    });

    group.bench_function("f32-simd-384d", |bench| {
        bench.iter(|| simd::dot_f32(black_box(&a), black_box(&b)))
    });

    let a_i8: Vec<i8> = (0..EMBED_DIM).map(|i| ((i % 127) as i8) - 63).collect();
    let b_i8: Vec<i8> = (0..EMBED_DIM).map(|i| (((i * 3) % 127) as i8) - 63).collect();

    group.bench_function("i8-scalar-384d", |bench| {
        bench.iter(|| simd::dot_i8_scalar(black_box(&a_i8), black_box(&b_i8)))
    });

    group.bench_function("i8-simd-384d", |bench| {
        bench.iter(|| simd::dot_i8(black_box(&a_i8), black_box(&b_i8)))
    });

    group.finish();
}

fn bench_quantization(c: &mut Criterion) {
    let mut group = c.benchmark_group("quantization");

    let n = 1000;
    let (tmp, _store) = build_store(n);
    let nd_search = NdArraySearch::from_sqlite(tmp.path()).unwrap();

    group.bench_function("build-quantized-1k", |b| {
        b.iter(|| {
            let _ = nd_search.to_quantized();
        })
    });

    group.bench_function("build-matryoshka-1k", |b| {
        b.iter(|| {
            let _ = nd_search.to_matryoshka(MatryoshkaConfig::default());
        })
    });

    group.bench_function("build-binary-1k", |b| {
        b.iter(|| {
            let _ = nd_search.to_binary(true);
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_search_strategies,
    bench_simd_dot_product,
    bench_quantization
);
criterion_main!(benches);
