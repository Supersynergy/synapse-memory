//! Criterion bench — distance→score transform: scalar vs turbo (SIMD via synapse-engine).
//!
//! Run:
//!   cargo bench -p synapse-core --bench dist_score --features turbo

use criterion::{Criterion, black_box, criterion_group, criterion_main};

fn dist_score_scalar(dists: &[f32]) -> Vec<f32> {
    dists.iter().map(|d| 1.0_f32 / (1.0_f32 + d)).collect()
}

fn bench_dist_score(c: &mut Criterion) {
    let n = 1000usize;
    let dists: Vec<f32> = (0..n).map(|i| (i as f32) * 0.001).collect();

    c.bench_function("dist_score_scalar 1k", |b| {
        b.iter(|| dist_score_scalar(black_box(&dists)))
    });

    #[cfg(feature = "turbo")]
    c.bench_function("dist_score_simd 1k (turbo)", |b| {
        b.iter(|| synapse_core::turbo::rrf_simd::distance_to_score(black_box(&dists)))
    });
}

criterion_group!(benches, bench_dist_score);
criterion_main!(benches);
