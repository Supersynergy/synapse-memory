//! In-process bench: NdArraySearch full-scan vs binary cascade.
//! Measures QPS and R@10 on the 168k corpus.
//!
//! Run:
//!   cargo run -p synapse-industry-bench --bin cascade_bench --release

use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::time::Instant;

fn slurp(path: &str) -> Vec<u8> {
    let mut b = Vec::new();
    File::open(path)
        .unwrap_or_else(|e| panic!("open {path}: {e}"))
        .read_to_end(&mut b)
        .unwrap();
    b
}

fn read_i64(buf: &[u8], off: usize) -> i64 {
    i64::from_le_bytes(buf[off..off + 8].try_into().unwrap())
}

fn load_f32_matrix(path: &str) -> (usize, usize, Vec<f32>) {
    let buf = slurp(path);
    let rows = read_i64(&buf, 0) as usize;
    let cols = read_i64(&buf, 8) as usize;
    let data = &buf[16..];
    assert_eq!(data.len(), rows * cols * 4);
    let v = data
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    (rows, cols, v)
}

fn load_i64_vec(path: &str) -> Vec<i64> {
    let buf = slurp(path);
    let n = read_i64(&buf, 0) as usize;
    let data = &buf[8..];
    assert_eq!(data.len(), n * 8);
    data.chunks_exact(8)
        .map(|b| i64::from_le_bytes(b.try_into().unwrap()))
        .collect()
}

fn load_gt(path: &str, k: usize) -> Vec<Vec<i64>> {
    let buf = slurp(path);
    let n_queries = read_i64(&buf, 0) as usize;
    let k_stored = read_i64(&buf, 8) as usize;
    let data = &buf[16..];
    (0..n_queries)
        .map(|i| {
            let start = i * k_stored * 8;
            let take = k.min(k_stored);
            data[start..start + take * 8]
                .chunks_exact(8)
                .map(|b| i64::from_le_bytes(b.try_into().unwrap()))
                .collect()
        })
        .collect()
}

fn recall_at_10(result_ids: &[i64], gt: &[i64]) -> f64 {
    let gt_set: HashSet<i64> = gt.iter().copied().collect();
    let hits = result_ids
        .iter()
        .take(10)
        .filter(|id| gt_set.contains(id))
        .count();
    hits as f64 / gt.len().min(10) as f64
}

fn main() {
    let corpus_vecs_path = std::env::var("CORPUS_VECS").unwrap_or("/tmp/corpus_vecs.bin".into());
    let corpus_ids_path = std::env::var("CORPUS_IDS").unwrap_or("/tmp/corpus_ids.bin".into());
    let query_vecs_path = std::env::var("QUERY_VECS").unwrap_or("/tmp/query_vecs.bin".into());
    let gt_path = std::env::var("GT_IDS").unwrap_or("/tmp/gt_ids.bin".into());

    eprintln!("Loading corpus…");
    let (n_corpus, dim, corpus_data) = load_f32_matrix(&corpus_vecs_path);
    let corpus_ids = load_i64_vec(&corpus_ids_path);
    assert_eq!(corpus_ids.len(), n_corpus);

    eprintln!("Loading queries + GT…");
    let (n_queries, q_dim, query_data) = load_f32_matrix(&query_vecs_path);
    assert_eq!(q_dim, dim);
    let gt = load_gt(&gt_path, 10);
    let n_queries = n_queries.min(gt.len()).min(500);

    eprintln!("Building NdArraySearch ({n_corpus} × {dim})…");
    use synapse_core::turbo::ndarray_search::NdArraySearch;
    let t_build = Instant::now();
    let idx =
        NdArraySearch::from_vecs(corpus_ids.clone(), corpus_data.clone(), dim).expect("from_vecs");
    eprintln!("Build: {:.2}s", t_build.elapsed().as_secs_f64());

    let k = 10usize;
    let n_warmup = 20;

    // ── Full scan bench ──────────────────────────────────────────────────────
    eprintln!("\n=== Full scan (search) ===");
    for i in 0..n_warmup {
        let q = &query_data[i * dim..(i + 1) * dim];
        let _ = idx.search(q, k);
    }
    let t0 = Instant::now();
    let mut recalls_full = Vec::with_capacity(n_queries);
    for i in 0..n_queries {
        let q = &query_data[i * dim..(i + 1) * dim];
        let results = idx.search(q, k);
        let ids: Vec<i64> = results.iter().map(|&(id, _)| id).collect();
        recalls_full.push(recall_at_10(&ids, &gt[i]));
    }
    let elapsed_full = t0.elapsed().as_secs_f64();
    let qps_full = n_queries as f64 / elapsed_full;
    let recall_full = recalls_full.iter().sum::<f64>() / recalls_full.len() as f64;
    println!(
        "full_scan  | QPS={qps_full:8.0} | R@10={recall_full:.4} | p50={:.2}ms",
        1000.0 * elapsed_full / n_queries as f64
    );

    // ── Cascade bench — various binary_k values ──────────────────────────────
    eprintln!("\n=== Binary cascade (search_cascade) ===");
    for binary_k in [512, 1024, 2048, 4096, 8192] {
        if binary_k > n_corpus {
            continue;
        }
        // warmup
        for i in 0..n_warmup {
            let q = &query_data[i * dim..(i + 1) * dim];
            let _ = idx.search_cascade(q, k, binary_k);
        }
        let t0 = Instant::now();
        let mut recalls = Vec::with_capacity(n_queries);
        for i in 0..n_queries {
            let q = &query_data[i * dim..(i + 1) * dim];
            let results = idx.search_cascade(q, k, binary_k);
            let ids: Vec<i64> = results.iter().map(|&(id, _)| id).collect();
            recalls.push(recall_at_10(&ids, &gt[i]));
        }
        let elapsed = t0.elapsed().as_secs_f64();
        let qps = n_queries as f64 / elapsed;
        let recall = recalls.iter().sum::<f64>() / recalls.len() as f64;
        println!(
            "cascade-{binary_k:<5} | QPS={qps:8.0} | R@10={recall:.4} | p50={:.2}ms",
            1000.0 * elapsed / n_queries as f64
        );
    }
}
