use criterion::{criterion_group, criterion_main, Criterion};
use ndarray::Array2;
use synapse_ultra::binary::{build_binary_matrix, pack_signs};
use synapse_ultra::search::{top_k_binary_first, top_k_f32};

fn make_random_matrix(n: usize, dim: usize) -> (Array2<f32>, Vec<u16>) {
    let data: Vec<f32> = (0..n * dim).map(|i| {
        let x = (i as f64 * 1.6180339887) % 1.0;
        (x * 2.0 - 1.0) as f32
    }).collect();
    let mut m = Array2::from_shape_vec((n, dim), data).unwrap();
    synapse_ultra::snapshot::normalize_rows(&mut m);
    let f16: Vec<u16> = m.iter().map(|&x| half::f16::from_f32(x).to_bits()).collect();
    (m, f16)
}

fn recall_at_k(
    queries: &[Vec<f32>],
    matrix: &Array2<f32>,
    matrix_f16: &[u16],
    bin: &[u8],
    n: usize,
    k: usize,
    rerank_n: usize,
) -> f64 {
    let mut total = 0usize;
    for q in queries {
        let q_sign = pack_signs(q);
        let strict = top_k_f32(q, matrix.view(), k);
        let binary = top_k_binary_first(q, &q_sign, bin, matrix_f16, n, k, rerank_n);
        let strict_set: std::collections::HashSet<usize> = strict.iter().map(|x| x.0).collect();
        total += binary.iter().filter(|x| strict_set.contains(&x.0)).count();
    }
    total as f64 / (queries.len() * k) as f64
}

fn bench_recall_levels(c: &mut Criterion) {
    let n = 1_000usize;
    let dim = 384usize;
    let (matrix, matrix_f16) = make_random_matrix(n, dim);
    let bin = build_binary_matrix(&matrix);

    let queries: Vec<Vec<f32>> = (0..20).map(|qi| {
        let mut q: Vec<f32> = (0..dim).map(|i| (((qi * 384 + i) as f64 * 2.7182818) % 1.0 * 2.0 - 1.0) as f32).collect();
        let norm = q.iter().map(|x| x * x).sum::<f32>().sqrt();
        q.iter_mut().for_each(|x| *x /= norm);
        q
    }).collect();

    // Print recall table
    println!("\nRecall@10 vs rerank_n (n=1k, 20 queries):");
    for &rn in &[50usize, 100, 200, 500] {
        let r = recall_at_k(&queries, &matrix, &matrix_f16, &bin, n, 10, rn);
        println!("  rerank_n={:4}: recall@10 = {:.3}", rn, r);
    }

    c.bench_function("recall_binary_first_rn500_20q_1k", |b| {
        b.iter(|| {
            for q in &queries {
                let q_sign = pack_signs(q);
                let _ = top_k_binary_first(q, &q_sign, &bin, &matrix_f16, n, 10, 500);
            }
        })
    });
}

criterion_group!(benches, bench_recall_levels);
criterion_main!(benches);
