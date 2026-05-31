//! Criterion bench — RRF merge: scalar vs turbo (SIMD via synapse-engine).
//!
//! Run:
//!   cargo bench -p synapse-core --bench rrf_simd --features turbo

use criterion::{Criterion, black_box, criterion_group, criterion_main};

fn rrf_scalar(ranks: &[f64], k: f64) -> Vec<f64> {
    ranks.iter().map(|&r| 1.0 / (k + r)).collect()
}

fn bench_rrf(c: &mut Criterion) {
    let n = 2000usize;
    let ranks: Vec<f64> = (1..=n).map(|i| i as f64).collect();
    let k = 60.0_f64;

    c.bench_function("rrf_scalar 2k", |b| {
        b.iter(|| rrf_scalar(black_box(&ranks), black_box(k)))
    });

    #[cfg(feature = "turbo")]
    c.bench_function("rrf_simd 2k (turbo)", |b| {
        b.iter(|| synapse_core::turbo::rrf_simd::reciprocal_ranks(black_box(&ranks), black_box(k)))
    });
}

criterion_group!(benches, bench_rrf);
criterion_main!(benches);
