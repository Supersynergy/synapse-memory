//! Standalone cascade bench: 10k corpus, 100 queries, p50/p99, R@10.
//! cargo run -p synapse-ann --release --features ann-usearch --example cascade_p50p99

#[cfg(feature = "ann-usearch")]
fn main() {
    use std::collections::HashSet;
    use std::time::Instant;
    use synapse_ann::{AnnIndex, UsearchIndex};

    const DIM: usize = 128;
    const N: usize = 10_000;
    const K: usize = 10;
    const QUERIES: usize = 100;

    fn vec(seed: u64) -> Vec<f32> {
        (0..DIM)
            .map(|i| {
                ((seed.wrapping_mul(13) + i as u64).wrapping_mul(7) % 997) as f32 / 997.0 - 0.5
            })
            .collect()
    }

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        1.0 - dot / (na * nb + 1e-9)
    }

    let vectors: Vec<Vec<f32>> = (0..N as u64).map(vec).collect();
    let mut idx = UsearchIndex::new(DIM, N).unwrap();
    for (i, v) in vectors.iter().enumerate() {
        idx.insert(i as u64, v).unwrap();
    }
    let queries: Vec<Vec<f32>> = (0..QUERIES as u64).map(|i| vec(0xdead + i)).collect();

    // brute-force truth
    let truth: Vec<Vec<u64>> = queries
        .iter()
        .map(|q| {
            let mut scored: Vec<(u64, f32)> = vectors
                .iter()
                .enumerate()
                .map(|(i, v)| (i as u64, cosine(q, v)))
                .collect();
            scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
            scored.into_iter().take(K).map(|(id, _)| id).collect()
        })
        .collect();

    // (a) ANN-only
    let mut lat_plain = Vec::with_capacity(QUERIES);
    let mut hits_plain = 0usize;
    for (q, t) in queries.iter().zip(truth.iter()) {
        let t0 = Instant::now();
        let res = idx.search(q, K).unwrap();
        lat_plain.push(t0.elapsed().as_micros());
        let t_set: HashSet<u64> = t.iter().copied().collect();
        hits_plain += res.iter().filter(|(id, _)| t_set.contains(id)).count();
    }

    // (b) cascade / guarantee (mult=4)
    let mut lat_rerank = Vec::with_capacity(QUERIES);
    let mut hits_rerank = 0usize;
    for (q, t) in queries.iter().zip(truth.iter()) {
        let t0 = Instant::now();
        let res = idx.search_with_rerank(q, K, 4).unwrap();
        lat_rerank.push(t0.elapsed().as_micros());
        let t_set: HashSet<u64> = t.iter().copied().collect();
        hits_rerank += res.iter().filter(|(id, _)| t_set.contains(id)).count();
    }

    fn percentile(v: &mut [u128], p: usize) -> u128 {
        v.sort();
        let idx = (v.len() * p / 100).min(v.len() - 1);
        v[idx]
    }

    let r10_plain = hits_plain as f64 / (QUERIES * K) as f64;
    let r10_rerank = hits_rerank as f64 / (QUERIES * K) as f64;

    println!("=== Cascade Bench: 10k corpus, 100 queries, K=10 ===");
    println!("(a) ANN-only:");
    println!("    R@10  = {:.4}", r10_plain);
    println!("    p50   = {} µs", percentile(&mut lat_plain, 50));
    println!("    p99   = {} µs", percentile(&mut lat_plain, 99));
    println!("(b) cascade --guarantee (mult=4x oversample):");
    println!("    R@10  = {:.4}", r10_rerank);
    println!("    p50   = {} µs", percentile(&mut lat_rerank, 50));
    println!("    p99   = {} µs", percentile(&mut lat_rerank, 99));
    println!("    delta R@10 = {:+.4}", r10_rerank - r10_plain);
}

#[cfg(not(feature = "ann-usearch"))]
fn main() {
    eprintln!("Requires --features ann-usearch");
}
