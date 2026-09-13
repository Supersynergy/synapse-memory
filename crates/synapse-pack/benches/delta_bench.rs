//! Benchmark: delta-pack + cache-stable token savings vs naive full pack.
//!
//! Run with: `cargo bench -p synapse-pack --bench delta_bench`.

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use synapse_pack::{Candidate, Kind, PackOptions, estimate_tokens, pack, pack_delta, render};

fn make_candidates(n: usize, tokens_each: usize) -> Vec<Candidate> {
    let line = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda. ";
    let per_line = estimate_tokens(line);
    let lines = (tokens_each / per_line.max(1)).max(1);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let mut text = String::new();
        for _ in 0..lines {
            text.push_str(line);
        }
        out.push(Candidate {
            id: i as i64,
            title: format!("fact-{i}"),
            text,
            score: 1.0 - (i as f32) * 0.01,
            kind: if i % 3 == 0 {
                Kind::KnownFact
            } else if i % 3 == 1 {
                Kind::Decision
            } else {
                Kind::File
            },
        });
    }
    out
}

fn bench_pack_scenarios(c: &mut Criterion) {
    let mut group = c.benchmark_group("pack");

    // Scenario A: full pack of 50 candidates @ 4000 token budget.
    let cands_a = make_candidates(50, 200);
    group.bench_with_input(
        BenchmarkId::new("full_50_at_4000", "naive"),
        &cands_a,
        |b, cands| {
            b.iter(|| {
                let opts = PackOptions {
                    budget_tokens: 4000,
                    header_reserve: 64,
                    ..PackOptions::default()
                };
                let p = pack(black_box(cands.clone()), &opts);
                black_box(p.used_tokens);
            })
        },
    );

    // Scenario B: delta pack where 40 of 50 candidates were already delivered.
    // prev_used_ids = [0..40], so only 10 new candidates survive the skip.
    let prev_ids: Vec<i64> = (0..40).collect();
    group.bench_with_input(
        BenchmarkId::new("delta_50_skip_40", "delta"),
        &cands_a,
        |b, cands| {
            b.iter(|| {
                let opts = PackOptions {
                    budget_tokens: 4000,
                    header_reserve: 64,
                    prev_used_ids: prev_ids.clone(),
                    cache_stable_order: false,
                };
                let p = pack_delta(black_box(cands.clone()), &opts);
                black_box(p.used_tokens);
            })
        },
    );

    // Scenario C: cache-stable ordering (same 50 candidates, 10 prev-used).
    let prev_ids_cs: Vec<i64> = (0..10).collect();
    group.bench_with_input(
        BenchmarkId::new("cache_stable_50", "cs"),
        &cands_a,
        |b, cands| {
            b.iter(|| {
                let opts = PackOptions {
                    budget_tokens: 4000,
                    header_reserve: 64,
                    prev_used_ids: prev_ids_cs.clone(),
                    cache_stable_order: true,
                };
                let p = pack(black_box(cands.clone()), &opts);
                black_box(p.used_tokens);
            })
        },
    );

    // Scenario D: render cost (should be tiny vs pack).
    let opts = PackOptions {
        budget_tokens: 4000,
        header_reserve: 64,
        ..PackOptions::default()
    };
    let packed = pack(cands_a.clone(), &opts);
    group.bench_function("render_50", |b| {
        b.iter(|| black_box(render(black_box(&packed))))
    });

    group.finish();
}

criterion_group!(benches, bench_pack_scenarios);
criterion_main!(benches);
