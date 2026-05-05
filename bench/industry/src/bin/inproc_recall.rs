//! In-process ANN recall bench — synapse-ann / usearch backend.
//!
//! Loads 168k text-embedding corpus + GT. Builds HNSW once per M value,
//! then sweeps ef_search via change_expansion_search (no rebuild).
//!
//! Run:
//!   cargo run -p synapse-industry-bench --bin inproc_recall --release \
//!     --features ann-usearch
//!
//! Data (export from bench/industry via Python):
//!   CORPUS_VECS=/tmp/corpus_vecs.bin  CORPUS_IDS=/tmp/corpus_ids.bin
//!   QUERY_VECS=/tmp/query_vecs.bin    GT_IDS=/tmp/gt_ids.bin

use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::time::Instant;
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

// ── I/O helpers ───────────────────────────────────────────────────────────────

fn read_i64(buf: &[u8], off: usize) -> i64 {
    i64::from_le_bytes(buf[off..off + 8].try_into().unwrap())
}

fn slurp(path: &str) -> Vec<u8> {
    let mut b = Vec::new();
    File::open(path)
        .unwrap_or_else(|e| panic!("open {path}: {e}"))
        .read_to_end(&mut b)
        .unwrap();
    b
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

// ── recall ────────────────────────────────────────────────────────────────────

fn recall_at_k(hits: &[(u64, f32)], gt_row: &[i64], k: usize) -> f64 {
    let ret: HashSet<u64> = hits.iter().take(k).map(|(id, _)| *id).collect();
    let gt: HashSet<u64> = gt_row.iter().take(k).map(|&v| v as u64).collect();
    if gt.is_empty() {
        return 1.0;
    }
    ret.intersection(&gt).count() as f64 / gt.len() as f64
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    let idx = ((sorted.len() as f64 * p).ceil() as usize).min(sorted.len() - 1);
    sorted[idx]
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
    let _corpus_ids = load_i64_vec(&corpus_ids_path);
    eprintln!("  {n_corpus} × {dim}");

    eprintln!("Loading queries + GT...");
    let (n_queries, _, query_vecs) = load_f32_matrix(&query_vecs_path);
    let (gt_rows, gt_cols, gt_matrix) = load_i64_matrix(&gt_ids_path);
    eprintln!("  {n_queries} queries, GT {gt_rows}×{gt_cols}");

    let ef_construct = 400usize;
    let ms: &[usize] = &[16, 32, 48];
    let ef_searches: &[usize] = &[64, 128, 200, 400];

    println!("# In-Process usearch sweep — {n_corpus} corpus × {n_queries} queries — dim={dim}");
    println!("# NO HTTP — direct Rust call into usearch HNSW");
    println!();
    println!(
        "{:<4} {:<5} {:<5} {:>8} {:>6} {:>7} {:>7} {:>7} {:>7}",
        "M", "ef_s", "ef_c", "QPS", "R@10", "R@100", "p50ms", "p95ms", "p99ms"
    );
    println!("{}", "-".repeat(68));

    let mut best_qps_at_98recall: Option<(usize, usize, f64, f64)> = None;

    for &m in ms {
        eprintln!("\nBuilding index M={m} ef_construct={ef_construct}...");
        let t0 = Instant::now();
        let opts = IndexOptions {
            dimensions: dim,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            connectivity: m,
            expansion_add: ef_construct,
            expansion_search: 64, // overridden per sweep
            multi: false,
        };
        let idx = Index::new(&opts).expect("usearch new");
        idx.reserve(n_corpus).expect("reserve");

        for i in 0..n_corpus {
            let v = &corpus_vecs[i * dim..(i + 1) * dim];
            idx.add(i as u64, v).expect("add");
        }
        let build_ms = t0.elapsed().as_millis();
        eprintln!("  built in {build_ms}ms");

        for &ef_s in ef_searches {
            idx.change_expansion_search(ef_s);

            let mut lats: Vec<f64> = Vec::with_capacity(n_queries);
            let mut r10s: Vec<f64> = Vec::with_capacity(n_queries);
            let mut r100s: Vec<f64> = Vec::with_capacity(n_queries);

            for qi in 0..n_queries {
                let qv = &query_vecs[qi * dim..(qi + 1) * dim];
                let gt_row = &gt_matrix[qi * gt_cols..(qi + 1) * gt_cols];

                let t = Instant::now();
                let res = idx.search(qv, 100).expect("search");
                lats.push(t.elapsed().as_secs_f64() * 1000.0);

                let hits: Vec<(u64, f32)> =
                    res.keys.into_iter().zip(res.distances).collect();
                r10s.push(recall_at_k(&hits, gt_row, 10));
                r100s.push(recall_at_k(&hits, gt_row, 100));
            }

            lats.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let mean_ms = lats.iter().sum::<f64>() / n_queries as f64;
            let qps = 1000.0 / mean_ms;
            let recall10 = r10s.iter().sum::<f64>() / n_queries as f64;
            let recall100 = r100s.iter().sum::<f64>() / n_queries as f64;

            println!(
                "{:<4} {:<5} {:<5} {:>8.0} {:>6.4} {:>7.4} {:>7.3} {:>7.3} {:>7.3}",
                m,
                ef_s,
                ef_construct,
                qps,
                recall10,
                recall100,
                pct(&lats, 0.50),
                pct(&lats, 0.95),
                pct(&lats, 0.99),
            );

            if recall10 >= 0.98 {
                if best_qps_at_98recall.map_or(true, |(_, _, q, _)| qps > q) {
                    best_qps_at_98recall = Some((m, ef_s, qps, recall10));
                }
            }
        }
    }

    println!();
    println!("# Baseline comparison (from iso_recall_99_sweep.md):");
    println!("# ultra_raw HTTP strict:        661 QPS @ R@10=0.920  (HTTP ~1.5ms overhead)");
    println!("# ultra_raw HTTP binary_first: 1510 QPS @ R@10=0.913");
    println!("# usearch in-proc M=16 ef=64:  5898 QPS @ R@10=0.919  (prior run)");
    println!("# usearch in-proc M=32 ef=400: 1078 QPS @ R@10=0.988  (prior run)");

    if let Some((m, ef_s, qps, r10)) = best_qps_at_98recall {
        println!();
        println!(
            "# Best in-proc ≥ R@10=0.98: M={m} ef_s={ef_s} → {qps:.0} QPS @ R@10={r10:.4}"
        );
        println!("# HTTP overhead removed → this is the true HNSW graph performance");
    } else {
        println!();
        println!("# No config reached R@10 ≥ 0.98 — check ef_construct or increase ef_search");
    }
}
