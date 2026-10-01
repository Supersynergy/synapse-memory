//! Criterion bench: RRF merge — scalar HashMap vs NEON sort-merge.
//!
//! Run:
//!   cargo bench -p synapse-core --bench rrf_neon

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use synapse_core::types::Hit;

fn make_hits(n: usize, id_offset: i64) -> Vec<Hit> {
    (0..n)
        .map(|i| Hit {
            id: id_offset + i as i64,
            uri: None,
            title: None,
            text: String::new(),
            score: 0.0,
            meta: None,
            ts: None,
        })
        .collect()
}

/// Scalar HashMap merge — baseline (original implementation).
fn rrf_merge_scalar(lex: Vec<Hit>, vec: Vec<Hit>, limit: usize) -> Vec<Hit> {
    let mut scores: std::collections::HashMap<i64, (f64, Hit)> = Default::default();
    let rrf_k = 60.0_f64;
    for (i, h) in lex.into_iter().enumerate() {
        let s = 1.0 / (rrf_k + (i + 1) as f64);
        scores
            .entry(h.id)
            .and_modify(|e| e.0 += s)
            .or_insert((s, h));
    }
    for (i, h) in vec.into_iter().enumerate() {
        let s = 1.0 / (rrf_k + (i + 1) as f64);
        scores
            .entry(h.id)
            .and_modify(|e| e.0 += s)
            .or_insert((s, h));
    }
    let mut out: Vec<_> = scores
        .into_values()
        .map(|(s, mut h)| {
            h.score = s;
            h
        })
        .collect();
    out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    out.truncate(limit);
    out
}

fn bench_rrf(c: &mut Criterion) {
    let mut g = c.benchmark_group("rrf_merge");

    for n in [256usize, 1024, 4096] {
        // Disjoint ids: worst case for sort-merge (no dedup savings).
        g.bench_with_input(BenchmarkId::new("scalar", n), &n, |b, &n| {
            b.iter(|| {
                let lex = make_hits(n, 0);
                let vec = make_hits(n, n as i64);
                rrf_merge_scalar(black_box(lex), black_box(vec), black_box(n / 2))
            })
        });

        g.bench_with_input(BenchmarkId::new("neon", n), &n, |b, &n| {
            b.iter(|| {
                let lex = make_hits(n, 0);
                let vec = make_hits(n, n as i64);
                synapse_core::db::rrf_merge_neon(black_box(lex), black_box(vec), black_box(n / 2))
            })
        });
    }

    g.finish();
}

criterion_group!(benches, bench_rrf);
criterion_main!(benches);
