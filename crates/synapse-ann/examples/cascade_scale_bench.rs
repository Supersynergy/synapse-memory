//! Scale bench: 100k + 1M corpus, 200 queries, K=10
//! cargo run -p synapse-ann --release --features ann-usearch --example cascade_scale_bench

#[cfg(feature = "ann-usearch")]
fn main() {
    use std::collections::HashSet;
    use std::time::Instant;
    use synapse_ann::{AnnIndex, UsearchIndex};

    const DIM: usize = 128;
    const K: usize = 10;
    const QUERIES: usize = 200;

    /// LCG-based pseudo-random float vector, normalized to unit sphere.
    fn vec(seed: u64) -> Vec<f32> {
        let mut state = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let mut v: Vec<f32> = (0..DIM)
            .map(|_| {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                // Box-Muller approx via uniform: map to [-1,1]
                (state >> 33) as f32 / (u32::MAX as f32 / 2.0) - 1.0
            })
            .collect();
        // L2-normalize so cosine = dot product
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
        v.iter_mut().for_each(|x| *x /= norm);
        v
    }

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        1.0 - dot / (na * nb + 1e-9)
    }

    fn percentile(v: &mut [u128], p: usize) -> u128 {
        v.sort();
        let idx = (v.len() * p / 100).min(v.len() - 1);
        v[idx]
    }

    fn run_bench(
        corpus_size: usize,
        ef_search: Option<usize>,
        vectors: &[Vec<f32>],
        queries: &[Vec<f32>],
        truth: &[Vec<u64>],
    ) {
        let n = corpus_size;
        println!("\n{}", "=".repeat(60));
        println!(
            "=== corpus={} ef_search={} ===",
            n,
            ef_search
                .map(|e| e.to_string())
                .unwrap_or_else(|| "default".into())
        );

        let t_build = Instant::now();
        let mut idx = UsearchIndex::new(DIM, n).unwrap();
        if let Some(ef) = ef_search {
            idx.set_expansion_search(ef);
        }
        for (i, v) in vectors[..n].iter().enumerate() {
            idx.insert(i as u64, v).unwrap();
        }
        let build_ms = t_build.elapsed().as_millis();
        println!("build: {}ms", build_ms);

        let mults: &[usize] = &[0, 4, 10, 50, 100]; // 0 = ANN-only

        for &mult in mults {
            let label = if mult == 0 {
                "ANN-only".to_string()
            } else {
                format!("cascade mult={}", mult)
            };

            let mut lats: Vec<u128> = Vec::with_capacity(QUERIES);
            let mut hits = 0usize;

            for (q, t) in queries.iter().zip(truth.iter()) {
                let t0 = Instant::now();
                let res = if mult == 0 {
                    idx.search(q, K).unwrap()
                } else {
                    idx.search_with_rerank(q, K, mult).unwrap()
                };
                lats.push(t0.elapsed().as_micros());
                let t_set: HashSet<u64> = t.iter().copied().collect();
                hits += res.iter().filter(|(id, _)| t_set.contains(id)).count();
            }

            let r10 = hits as f64 / (QUERIES * K) as f64;
            let p50 = percentile(&mut lats, 50);
            let p95 = percentile(&mut lats, 95);
            let p99 = percentile(&mut lats, 99);
            println!(
                "  {:20}  R@10={:.4}  p50={:5}µs  p95={:6}µs  p99={:6}µs",
                label, r10, p50, p95, p99
            );
        }
    }

    // ---- 100k bench ----
    let n100k: usize = 100_000;
    println!("Generating {} vectors...", n100k);
    let vectors_100k: Vec<Vec<f32>> = (0..n100k as u64).map(vec).collect();
    // Query seeds chosen so they land in same distribution as corpus (uniform LCG).
    // Offset by n100k so they are NOT in corpus but draw from same distribution.
    let queries: Vec<Vec<f32>> = (0..QUERIES as u64)
        .map(|i| vec(n100k as u64 + 1337 + i))
        .collect();

    println!("Computing brute-force truth for 100k...");
    let t_truth = Instant::now();
    let truth_100k: Vec<Vec<u64>> = queries
        .iter()
        .map(|q| {
            let mut scored: Vec<(u64, f32)> = vectors_100k
                .iter()
                .enumerate()
                .map(|(i, v)| (i as u64, cosine(q, v)))
                .collect();
            scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
            scored.into_iter().take(K).map(|(id, _)| id).collect()
        })
        .collect();
    println!("truth computed in {}ms", t_truth.elapsed().as_millis());

    // default ef_search
    run_bench(n100k, None, &vectors_100k, &queries, &truth_100k);
    // low ef_search=32 to force recall gap
    run_bench(n100k, Some(32), &vectors_100k, &queries, &truth_100k);
    // ef=256
    run_bench(n100k, Some(256), &vectors_100k, &queries, &truth_100k);

    // ---- 1M bench ----
    let n1m: usize = 1_000_000;
    let deadline = std::time::Duration::from_secs(1800); // 30min hard cap
    let t_start_1m = Instant::now();

    println!("\nGenerating {} vectors (this may take a while)...", n1m);
    // Reuse 100k, extend with fresh vecs
    let mut vectors_1m = vectors_100k;
    vectors_1m.reserve(n1m - vectors_1m.len());
    for i in n100k as u64..n1m as u64 {
        vectors_1m.push(vec(i));
    }

    if t_start_1m.elapsed() > deadline {
        println!("SKIP 1M: generation took too long");
    } else {
        // For 1M: brute-force truth too slow (~200*1M cosine = 200M ops).
        // Use approximate truth: build a high-ef index and use that as proxy.
        println!("Computing approx-truth for 1M (high-ef HNSW)...");
        let mut idx_truth = UsearchIndex::new(DIM, n1m).unwrap();
        idx_truth.set_expansion_search(16384);
        for (i, v) in vectors_1m.iter().enumerate() {
            idx_truth.insert(i as u64, v).unwrap();
        }
        let truth_1m: Vec<Vec<u64>> = queries
            .iter()
            .map(|q| {
                idx_truth
                    .search(q, K)
                    .unwrap()
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect()
            })
            .collect();

        if t_start_1m.elapsed() > deadline {
            println!("SKIP 1M bench: deadline exceeded after truth build");
        } else {
            // default ef
            run_bench(n1m, None, &vectors_1m, &queries, &truth_1m);
            if t_start_1m.elapsed() < deadline {
                // low ef=32 (force gap)
                run_bench(n1m, Some(32), &vectors_1m, &queries, &truth_1m);
            }
            if t_start_1m.elapsed() < deadline {
                // ef=256
                run_bench(n1m, Some(256), &vectors_1m, &queries, &truth_1m);
            }
            if t_start_1m.elapsed() < deadline {
                // ef=16384
                run_bench(n1m, Some(16384), &vectors_1m, &queries, &truth_1m);
            }
        }
    }

    println!("\nDone.");
}

#[cfg(not(feature = "ann-usearch"))]
fn main() {
    eprintln!("Requires --features ann-usearch");
}
