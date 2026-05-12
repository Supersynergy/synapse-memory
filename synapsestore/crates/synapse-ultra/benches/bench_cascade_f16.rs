/// Benchmark: cascade search (hamming → f16 rerank) across 1k/10k/100k synthetic vectors.
/// Uses xorshift for deterministic normalized 384-d vectors.
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use synapse_ultra::search::{self, query_to_f16};

// ── xorshift PRNG ────────────────────────────────────────────────────────────

fn xorshift64(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

fn rand_f32(state: &mut u64) -> f32 {
    // map u64 to [-1, 1)
    let bits = xorshift64(state);
    (bits as f32 / u64::MAX as f32) * 2.0 - 1.0
}

/// Build n×384 normalized f32 matrix + packed binary + f16 flat buffer.
fn build_corpus(n: usize) -> (Vec<f32>, Vec<u8>, Vec<u16>) {
    let dim = 384usize;
    let mut state: u64 = 0xdeadbeef_cafebabe;
    let mut matrix_f32 = Vec::with_capacity(n * dim);
    let mut bin_matrix = Vec::with_capacity(n * 48);
    let mut matrix_f16 = Vec::with_capacity(n * dim);

    for _ in 0..n {
        // generate raw row
        let mut row: Vec<f32> = (0..dim).map(|_| rand_f32(&mut state)).collect();
        // L2-normalize
        let norm: f32 = row.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
        row.iter_mut().for_each(|x| *x /= norm);

        // pack signs (48 bytes = 384 bits)
        let mut sign_bytes = [0u8; 48];
        for (i, &v) in row.iter().enumerate() {
            if v >= 0.0 {
                sign_bytes[i / 8] |= 1 << (i % 8);
            }
        }
        bin_matrix.extend_from_slice(&sign_bytes);

        // f16
        for &v in &row {
            matrix_f16.push(half::f16::from_f32(v).to_bits());
        }

        matrix_f32.extend_from_slice(&row);
    }

    (matrix_f32, bin_matrix, matrix_f16)
}

fn build_query(seed: u64) -> Vec<f32> {
    let dim = 384usize;
    let mut state = seed;
    let mut q: Vec<f32> = (0..dim).map(|_| rand_f32(&mut state)).collect();
    let norm: f32 = q.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    q.iter_mut().for_each(|x| *x /= norm);
    q
}

fn pack_signs(q: &[f32]) -> Vec<u8> {
    let mut out = vec![0u8; 48];
    for (i, &v) in q.iter().enumerate() {
        if v >= 0.0 {
            out[i / 8] |= 1 << (i % 8);
        }
    }
    out
}

// ── benchmarks ───────────────────────────────────────────────────────────────

fn bench_cascade(c: &mut Criterion) {
    let mut group = c.benchmark_group("cascade_f16");

    for &n in &[1_000usize, 10_000, 100_000] {
        let (matrix_f32, bin_matrix, matrix_f16) = build_corpus(n);
        let query = build_query(0xf00dface_babe1234);
        let query_sign = pack_signs(&query);
        let k = 10;
        let rerank_n = search::DEFAULT_BINARY_RERANK.max(k * 16);

        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::new("hamming_rerank_f16", n), &n, |b, _| {
            b.iter(|| {
                search::top_k_binary_first(
                    &query,
                    &query_sign,
                    &bin_matrix,
                    &matrix_f16,
                    n,
                    k,
                    rerank_n,
                )
            });
        });

        // Also bench the f16 query conversion (should be ~negligible)
        group.bench_with_input(BenchmarkId::new("query_to_f16_only", n), &n, |b, _| {
            b.iter(|| query_to_f16(&query));
        });
    }

    group.finish();
}

criterion_group!(benches, bench_cascade);
criterion_main!(benches);
