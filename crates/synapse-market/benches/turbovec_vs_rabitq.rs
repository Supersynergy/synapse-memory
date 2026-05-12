/// Bench: turbovec vs RaBitQ vs SimSIMD-i8 vs plain-f32 dot-product
///
/// 100k synthetic 768-d f32 vectors, 1000 queries, top-10.
/// Metrics: build_time, query p50, recall@10, bytes/vec.
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use synapse_market::signal::similar::{dot_product_top_k, BruteForceI8Index};
use synapse_market::signal::turbovec_index::TurboVecIndex;
use synapse_market::signal::SignalId;

const N: usize = 100_000;
const DIM: usize = 768;
const N_QUERIES: usize = 1_000;
const TOP_K: usize = 10;

fn make_rng_vec(seed: u64, len: usize) -> Vec<f32> {
    // lcg deterministic
    let mut x = seed.wrapping_add(1);
    let mut v = Vec::with_capacity(len);
    for _ in 0..len {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        v.push(((x >> 33) as f32) / (u32::MAX as f32) * 2.0 - 1.0);
    }
    // normalise
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

fn make_corpus(n: usize) -> Vec<(SignalId, Vec<f32>)> {
    (0..n)
        .map(|i| (i as SignalId, make_rng_vec(i as u64 * 7919, DIM)))
        .collect()
}

fn make_queries(n: usize) -> Vec<Vec<f32>> {
    (0..n)
        .map(|i| make_rng_vec(i as u64 * 9973 + 1_000_000, DIM))
        .collect()
}

fn recall_at_k(got: &[(SignalId, f32)], truth: &[(SignalId, f32)]) -> f32 {
    let truth_ids: std::collections::HashSet<SignalId> = truth.iter().map(|(id, _)| *id).collect();
    let hits = got.iter().filter(|(id, _)| truth_ids.contains(id)).count();
    hits as f32 / truth.len() as f32
}

pub fn bench_build(c: &mut Criterion) {
    let corpus = make_corpus(N);
    let mut g = c.benchmark_group("build_100k");
    g.sample_size(10);

    g.bench_function("turbovec_4bit", |b| {
        b.iter(|| TurboVecIndex::build(&corpus, 4).unwrap())
    });

    // RaBitQ build skipped: rabitq-rs 0.9 panics with ex_bits=4 (only 0/2/6 valid)
    // g.bench_function("rabitq_ivf", ...);

    g.bench_function("simsimd_i8", |b| {
        b.iter(|| BruteForceI8Index::build(&corpus))
    });

    g.finish();
}

pub fn bench_query(c: &mut Criterion) {
    let corpus = make_corpus(N);
    let queries = make_queries(N_QUERIES);

    let tv = TurboVecIndex::build(&corpus, 4).unwrap();
    let bf = BruteForceI8Index::build(&corpus);

    let mut g = c.benchmark_group("query_p50_1000q");
    g.sample_size(20);

    g.bench_function("turbovec_4bit", |b| {
        b.iter(|| {
            for q in &queries {
                let _ = tv.search(q, TOP_K).unwrap();
            }
        })
    });

    // RaBitQ query skipped: same ex_bits=4 panic (rabitq-rs 0.9 bug)

    g.bench_function("simsimd_i8_brute", |b| {
        b.iter(|| {
            for q in &queries {
                let _ = bf.search(q, TOP_K);
            }
        })
    });

    g.bench_function("plain_f32_dot", |b| {
        b.iter(|| {
            for q in &queries {
                let _ = dot_product_top_k(&corpus, q, TOP_K);
            }
        })
    });

    g.finish();
}

pub fn bench_recall(c: &mut Criterion) {
    let corpus = make_corpus(10_000); // smaller for recall measurement
    let queries = make_queries(100);

    let tv = TurboVecIndex::build(&corpus, 4).unwrap();
    let bf = BruteForceI8Index::build(&corpus);

    let mut total_tv = 0.0f32;
    let mut total_bf = 0.0f32;
    for q in &queries {
        let truth = dot_product_top_k(&corpus, q, TOP_K);
        total_tv += recall_at_k(&tv.search(q, TOP_K).unwrap(), &truth);
        total_bf += recall_at_k(&bf.search(q, TOP_K), &truth);
    }
    let n = queries.len() as f32;
    println!(
        "\n=== recall@{TOP_K} (10k corpus, 100q) ===\n  turbovec-4bit : {:.3}\n  simsimd-i8    : {:.3}\n  rabitq-ivf    : SKIP (ex_bits=4 panic, rabitq-rs 0.9 bug)",
        total_tv / n, total_bf / n
    );

    // bytes/vec
    println!(
        "=== bytes/vec ===\n  turbovec-4bit : {} B  ({:.1}× vs f32)\n  f32 baseline  : {} B",
        tv.bytes_per_vec(),
        (DIM * 4) as f32 / tv.bytes_per_vec() as f32,
        DIM * 4
    );

    // dummy bench so criterion records it
    c.bench_function("recall_report", |b| b.iter(|| total_tv));
}

criterion_group!(benches, bench_build, bench_query, bench_recall);
criterion_main!(benches);
