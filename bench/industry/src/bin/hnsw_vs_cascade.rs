//! Head-to-head: usearch HNSW vs NdArray cascade (own implementation).
//!
//! Runs both backends on the same 168k corpus, same query set, same GT.
//! Produces a comparison table: own_qps / own_recall / usearch_qps / usearch_recall / build_time.
//!
//! Run:
//!   cargo run -p synapse-industry-bench --bin hnsw_vs_cascade --release \
//!     --features ann-usearch

use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::time::Instant;
use synapse_core::turbo::ndarray_search::NdArraySearch;
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

// ── I/O helpers ───────────────────────────────────────────────────────────────

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

fn load_i64_matrix(path: &str) -> (usize, usize, Vec<i64>) {
    let buf = slurp(path);
    let rows = read_i64(&buf, 0) as usize;
    let cols = read_i64(&buf, 8) as usize;
    let data = &buf[16..];
    assert_eq!(data.len(), rows * cols * 8);
    let v = data
        .chunks_exact(8)
        .map(|b| i64::from_le_bytes(b.try_into().unwrap()))
        .collect();
    (rows, cols, v)
}

// ── metrics ───────────────────────────────────────────────────────────────────

fn recall_at_k_i64(result_ids: &[i64], gt_row: &[i64], k: usize) -> f64 {
    let gt: HashSet<i64> = gt_row.iter().take(k).copied().collect();
    if gt.is_empty() {
        return 1.0;
    }
    result_ids
        .iter()
        .take(k)
        .filter(|id| gt.contains(id))
        .count() as f64
        / gt.len() as f64
}

fn recall_at_k_u64(hits: &[(u64, f32)], gt_row: &[i64], k: usize) -> f64 {
    let gt: HashSet<u64> = gt_row.iter().take(k).map(|&v| v as u64).collect();
    if gt.is_empty() {
        return 1.0;
    }
    hits.iter()
        .take(k)
        .filter(|(id, _)| gt.contains(id))
        .count() as f64
        / gt.len() as f64
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    let idx = ((sorted.len() as f64 * p).ceil() as usize).min(sorted.len() - 1);
    sorted[idx]
}

fn median_ms(lats_ms: &[f64]) -> f64 {
    let mut s = lats_ms.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    pct(&s, 0.50)
}

// ── cascade bench ─────────────────────────────────────────────────────────────

struct CascadeResult {
    binary_k: usize,
    qps: f64,
    recall10: f64,
    p50ms: f64,
}

fn bench_cascade(
    idx: &NdArraySearch,
    query_vecs: &[f32],
    gt_matrix: &[i64],
    n_queries: usize,
    dim: usize,
    gt_cols: usize,
    binary_k: usize,
    k: usize,
) -> CascadeResult {
    let n_warmup = 20;
    for i in 0..n_warmup {
        let q = &query_vecs[i * dim..(i + 1) * dim];
        let _ = idx.search_cascade(q, k, binary_k);
    }
    let mut lats: Vec<f64> = Vec::with_capacity(n_queries);
    let mut r10s: Vec<f64> = Vec::with_capacity(n_queries);
    for qi in 0..n_queries {
        let q = &query_vecs[qi * dim..(qi + 1) * dim];
        let gt_row = &gt_matrix[qi * gt_cols..(qi + 1) * gt_cols];
        let t = Instant::now();
        let results = idx.search_cascade(q, k, binary_k);
        lats.push(t.elapsed().as_secs_f64() * 1000.0);
        let ids: Vec<i64> = results.iter().map(|&(id, _)| id).collect();
        r10s.push(recall_at_k_i64(&ids, gt_row, k));
    }
    let mean_ms = lats.iter().sum::<f64>() / n_queries as f64;
    CascadeResult {
        binary_k,
        qps: 1000.0 / mean_ms,
        recall10: r10s.iter().sum::<f64>() / n_queries as f64,
        p50ms: median_ms(&lats),
    }
}

// ── usearch bench ─────────────────────────────────────────────────────────────

struct UsearchResult {
    m: usize,
    ef_s: usize,
    ef_c: usize,
    build_ms: u128,
    qps: f64,
    recall10: f64,
    p50ms: f64,
}

/// Build usearch index once for M/ef_c, return (index, build_ms).
fn build_usearch_index(
    corpus_vecs: &[f32],
    corpus_ids: &[i64],
    n_corpus: usize,
    dim: usize,
    m: usize,
    ef_c: usize,
) -> (Index, u128) {
    let t0 = Instant::now();
    let opts = IndexOptions {
        dimensions: dim,
        metric: MetricKind::Cos,
        quantization: ScalarKind::F32,
        connectivity: m,
        expansion_add: ef_c,
        expansion_search: 64,
        multi: false,
    };
    let idx = Index::new(&opts).expect("usearch new");
    idx.reserve(n_corpus).expect("reserve");
    for i in 0..n_corpus {
        let v = &corpus_vecs[i * dim..(i + 1) * dim];
        idx.add(corpus_ids[i] as u64, v).expect("add");
    }
    (idx, t0.elapsed().as_millis())
}

/// Query sweep using pre-built index — no rebuild.
fn bench_usearch_sweep(
    idx: &Index,
    query_vecs: &[f32],
    gt_matrix: &[i64],
    n_queries: usize,
    dim: usize,
    gt_cols: usize,
    m: usize,
    ef_c: usize,
    build_ms: u128,
    ef_s: usize,
    k: usize,
) -> UsearchResult {
    idx.change_expansion_search(ef_s);
    let n_warmup = 20;
    for i in 0..n_warmup {
        let q = &query_vecs[i * dim..(i + 1) * dim];
        let _ = idx.search(q, k).expect("search");
    }
    let mut lats: Vec<f64> = Vec::with_capacity(n_queries);
    let mut r10s: Vec<f64> = Vec::with_capacity(n_queries);
    for qi in 0..n_queries {
        let q = &query_vecs[qi * dim..(qi + 1) * dim];
        let gt_row = &gt_matrix[qi * gt_cols..(qi + 1) * gt_cols];
        let t = Instant::now();
        let res = idx.search(q, k).expect("search");
        lats.push(t.elapsed().as_secs_f64() * 1000.0);
        let hits: Vec<(u64, f32)> = res.keys.into_iter().zip(res.distances).collect();
        r10s.push(recall_at_k_u64(&hits, gt_row, k));
    }
    let mean_ms = lats.iter().sum::<f64>() / n_queries as f64;
    UsearchResult {
        m,
        ef_s,
        ef_c,
        build_ms,
        qps: 1000.0 / mean_ms,
        recall10: r10s.iter().sum::<f64>() / n_queries as f64,
        p50ms: median_ms(&lats),
    }
}

// ── main ──────────────────────────────────────────────────────────────────────

fn main() {
    let corpus_vecs_path =
        std::env::var("CORPUS_VECS").unwrap_or_else(|_| "/tmp/corpus_vecs.bin".into());
    let corpus_ids_path =
        std::env::var("CORPUS_IDS").unwrap_or_else(|_| "/tmp/corpus_ids.bin".into());
    let query_vecs_path =
        std::env::var("QUERY_VECS").unwrap_or_else(|_| "/tmp/query_vecs.bin".into());
    let gt_ids_path = std::env::var("GT_IDS").unwrap_or_else(|_| "/tmp/gt_ids.bin".into());

    eprintln!("Loading corpus...");
    let (n_corpus, dim, corpus_vecs) = load_f32_matrix(&corpus_vecs_path);
    let corpus_ids = load_i64_vec(&corpus_ids_path);
    eprintln!("  {n_corpus} × {dim}");

    eprintln!("Loading queries + GT...");
    let (n_queries_full, _, query_vecs) = load_f32_matrix(&query_vecs_path);
    let (gt_rows, gt_cols, gt_matrix) = load_i64_matrix(&gt_ids_path);
    let n_queries = n_queries_full.min(gt_rows).min(1000);
    eprintln!("  {n_queries} queries, GT {gt_rows}×{gt_cols}");

    let k = 10usize;

    // ── Build NdArraySearch (cascade / own ndarray backend) ──────────────────
    eprintln!("\nBuilding NdArraySearch (own binary-cascade backend)...");
    let t_build = Instant::now();
    let cascade_idx = NdArraySearch::from_vecs(corpus_ids.clone(), corpus_vecs.clone(), dim)
        .expect("NdArraySearch::from_vecs");
    let cascade_build_ms = t_build.elapsed().as_millis();
    eprintln!("  cascade build: {cascade_build_ms}ms");

    // ── Cascade sweep ─────────────────────────────────────────────────────────
    println!("\n## Cascade (own NdArray + binary int8 pre-filter)");
    println!("build_time: {cascade_build_ms}ms");
    println!();
    println!(
        "{:<12} {:>8} {:>8} {:>8}",
        "binary_k", "QPS", "R@10", "p50ms"
    );
    println!("{}", "-".repeat(42));

    let binary_ks: &[usize] = &[512, 1024, 2048, 4096, 8192];
    let mut cascade_results = Vec::new();
    for &bk in binary_ks {
        if bk > n_corpus {
            continue;
        }
        let r = bench_cascade(
            &cascade_idx,
            &query_vecs,
            &gt_matrix,
            n_queries,
            dim,
            gt_cols,
            bk,
            k,
        );
        println!(
            "{:<12} {:>8.0} {:>8.4} {:>8.3}",
            r.binary_k, r.qps, r.recall10, r.p50ms
        );
        cascade_results.push(r);
    }

    // ── usearch sweep ─────────────────────────────────────────────────────────
    let ms: &[usize] = &[16, 32, 48];
    let ef_searches: &[usize] = &[32, 64, 128, 200];
    let ef_c = 400usize;

    println!("\n## usearch HNSW (via synapse-ann)");
    println!();
    println!(
        "{:<4} {:<5} {:<5} {:>10} {:>8} {:>8} {:>8}",
        "M", "ef_s", "ef_c", "build_ms", "QPS", "R@10", "p50ms"
    );
    println!("{}", "-".repeat(58));

    let mut usearch_results = Vec::new();
    for &m in ms {
        eprintln!("\nBuilding usearch M={m} ef_c={ef_c}...");
        let (us_idx, build_ms) =
            build_usearch_index(&corpus_vecs, &corpus_ids, n_corpus, dim, m, ef_c);
        eprintln!("  built in {build_ms}ms");
        for &ef_s in ef_searches {
            let r = bench_usearch_sweep(
                &us_idx,
                &query_vecs,
                &gt_matrix,
                n_queries,
                dim,
                gt_cols,
                m,
                ef_c,
                build_ms,
                ef_s,
                k,
            );
            println!(
                "{:<4} {:<5} {:<5} {:>10} {:>8.0} {:>8.4} {:>8.3}",
                r.m, r.ef_s, r.ef_c, r.build_ms, r.qps, r.recall10, r.p50ms
            );
            usearch_results.push(r);
        }
    }

    // ── Head-to-head comparison table ─────────────────────────────────────────
    println!("\n## Head-to-head: usearch vs cascade at iso-recall brackets");
    println!();
    println!(
        "{:<16} {:>8} {:>8}  {:<16} {:>8} {:>8}",
        "cascade_config", "cas_QPS", "cas_R10", "usearch_config", "us_QPS", "us_R10"
    );
    println!("{}", "-".repeat(72));

    // pair nearest cascade recall to usearch recall
    let brackets = [(0.94, 0.96), (0.97, 0.985), (0.985, 0.995)];
    for (lo, hi) in brackets {
        let cas = cascade_results
            .iter()
            .filter(|r| r.recall10 >= lo && r.recall10 < hi)
            .max_by(|a, b| a.qps.partial_cmp(&b.qps).unwrap());
        let us = usearch_results
            .iter()
            .filter(|r| r.recall10 >= lo && r.recall10 < hi)
            .max_by(|a, b| a.qps.partial_cmp(&b.qps).unwrap());
        let cas_s = cas
            .map(|r| format!("cascade-{}", r.binary_k))
            .unwrap_or_else(|| "none".into());
        let cas_qps = cas.map(|r| format!("{:.0}", r.qps)).unwrap_or("-".into());
        let cas_r = cas
            .map(|r| format!("{:.4}", r.recall10))
            .unwrap_or("-".into());
        let us_s = us
            .map(|r| format!("M={} ef={}", r.m, r.ef_s))
            .unwrap_or_else(|| "none".into());
        let us_qps = us.map(|r| format!("{:.0}", r.qps)).unwrap_or("-".into());
        let us_r = us
            .map(|r| format!("{:.4}", r.recall10))
            .unwrap_or("-".into());
        println!(
            "{:<16} {:>8} {:>8}  {:<16} {:>8} {:>8}",
            cas_s, cas_qps, cas_r, us_s, us_qps, us_r
        );
    }

    println!();
    println!("# Summary:");
    println!("# cascade build: {cascade_build_ms}ms | usearch build (M=48 ef_c={ef_c}): see table");
    println!("# cascade = NdArray + binary int8 pre-filter (pure Rust, no C++ dep)");
    println!("# usearch  = usearch 2.25 C++ HNSW via synapse-ann crate");
}
