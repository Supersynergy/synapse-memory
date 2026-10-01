//! Raw ANN micro-bench: usearch (HNSW) vs FAISS-Flat / FAISS-HNSW.
//! In-memory only, no SQLite/FTS/graph/rerank overhead.
//! 10k × 384-dim f32, 100 queries, top-10.
//!
//! Run:
//!   cargo run -p synapse-ann --release --features ann-usearch --example raw_ann_microbench
//!
//! FAISS leg: writes /tmp/raw_ann_data.npy, calls python3 /tmp/raw_ann_faiss.py,
//! parses stdout "FLAT_P50=... FLAT_P99=... HNSW_P50=... HNSW_P99=...
//!                FLAT_R10=... HNSW_R10=...".
//! If python/faiss unavailable → FAISS row shows N/A.

#[cfg(not(feature = "ann-usearch"))]
fn main() {
    eprintln!("Requires --features ann-usearch");
    std::process::exit(1);
}

#[cfg(feature = "ann-usearch")]
fn main() {
    use std::collections::HashSet;
    use std::time::Instant;
    use synapse_ann::{AnnIndex, UsearchIndex};

    const DIM: usize = 384;
    const N: usize = 10_000;
    const K: usize = 10;
    const QUERIES: usize = 100;

    // ── deterministic pseudo-random vectors ──────────────────────────────────
    fn make_vec(seed: u64, dim: usize) -> Vec<f32> {
        let mut v: Vec<f32> = (0..dim)
            .map(|i| {
                let h = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(i as u64 * 1442695040888963407);
                (h as i64 as f32) / (i64::MAX as f32)
            })
            .collect();
        // L2-normalize so cosine == dot product
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
        v.iter_mut().for_each(|x| *x /= norm);
        v
    }

    fn cosine_dist(a: &[f32], b: &[f32]) -> f32 {
        1.0 - a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>()
    }

    // ── brute-force ground-truth ─────────────────────────────────────────────
    fn brute_top_k(corpus: &[Vec<f32>], q: &[f32], k: usize) -> Vec<u64> {
        let mut dists: Vec<(usize, f32)> = corpus
            .iter()
            .enumerate()
            .map(|(i, v)| (i, cosine_dist(v, q)))
            .collect();
        dists.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        dists.truncate(k);
        dists.iter().map(|(i, _)| *i as u64).collect()
    }

    fn recall(got: &[(u64, f32)], truth: &[u64]) -> f32 {
        let truth_set: HashSet<u64> = truth.iter().copied().collect();
        let hits = got.iter().filter(|(id, _)| truth_set.contains(id)).count();
        hits as f32 / truth.len() as f32
    }

    fn percentile(mut v: Vec<u128>, p: f64) -> u128 {
        v.sort_unstable();
        let idx = ((v.len() as f64 * p / 100.0).ceil() as usize).saturating_sub(1);
        v[idx.min(v.len() - 1)]
    }

    println!("Building corpus: N={N} DIM={DIM}");
    let corpus: Vec<Vec<f32>> = (0..N as u64).map(|s| make_vec(s, DIM)).collect();
    let queries: Vec<Vec<f32>> = (0..QUERIES as u64)
        .map(|s| make_vec(s + 0xC0_FFEE, DIM))
        .collect();
    let truths: Vec<Vec<u64>> = queries.iter().map(|q| brute_top_k(&corpus, q, K)).collect();

    // ── usearch HNSW ─────────────────────────────────────────────────────────
    print!("Building usearch index... ");
    let build_t0 = Instant::now();
    let mut idx = UsearchIndex::new(DIM, N).unwrap();
    for (i, v) in corpus.iter().enumerate() {
        idx.insert(i as u64, v).unwrap();
    }
    let build_ms = build_t0.elapsed().as_millis();
    println!("{build_ms}ms");

    // warm-up
    for q in queries.iter().take(5) {
        let _ = idx.search(q, K).unwrap();
    }

    let mut us_latencies: Vec<u128> = Vec::with_capacity(QUERIES);
    let mut us_recalls: Vec<f32> = Vec::with_capacity(QUERIES);
    for (q, truth) in queries.iter().zip(truths.iter()) {
        let t0 = Instant::now();
        let res = idx.search(q, K).unwrap();
        us_latencies.push(t0.elapsed().as_micros());
        us_recalls.push(recall(&res, truth));
    }
    let us_p50 = percentile(us_latencies.clone(), 50.0);
    let us_p99 = percentile(us_latencies.clone(), 99.0);
    let us_r10: f32 = us_recalls.iter().sum::<f32>() / QUERIES as f32;

    // ── write data for FAISS leg ──────────────────────────────────────────────
    let npy_path = "/tmp/raw_ann_data.npy";
    let qry_path = "/tmp/raw_ann_queries.npy";
    let py_path = "/tmp/raw_ann_faiss.py";

    write_npy_f32_2d(npy_path, &corpus, DIM);
    write_npy_f32_2d(qry_path, &queries, DIM);
    write_truths("/tmp/raw_ann_truths.txt", &truths);
    write_faiss_script(py_path, N, DIM, K, QUERIES);

    // ── invoke python faiss ───────────────────────────────────────────────────
    let (flat_p50, flat_p99, flat_r10, hnsw_p50, hnsw_p99, hnsw_r10) = run_faiss_script(py_path);

    // ── print table ──────────────────────────────────────────────────────────
    println!();
    println!("═══════════════════════════════════════════════════════════════");
    println!("  RAW ANN MICRO-BENCH  N={N}  DIM={DIM}  K={K}  Q={QUERIES}");
    println!("═══════════════════════════════════════════════════════════════");
    println!(
        "{:<22} {:>10} {:>10} {:>8}",
        "Backend", "p50 µs", "p99 µs", "R@10"
    );
    println!("{}", "─".repeat(55));
    println!(
        "{:<22} {:>10} {:>10} {:>8.3}",
        "FAISS-Flat (exact)", flat_p50, flat_p99, flat_r10
    );
    println!(
        "{:<22} {:>10} {:>10} {:>8.3}",
        "FAISS-HNSW", hnsw_p50, hnsw_p99, hnsw_r10
    );
    println!(
        "{:<22} {:>10} {:>10} {:>8.3}",
        "usearch-HNSW (synapse)", us_p50, us_p99, us_r10
    );
    println!("{}", "─".repeat(55));
    println!("build_ms={build_ms}  (usearch, N={N})");
    println!("═══════════════════════════════════════════════════════════════");

    // ── parity claim ─────────────────────────────────────────────────────────
    if us_r10 >= 0.95 {
        println!("PARITY: usearch R@10={:.3} ≥ 0.95 ✓", us_r10);
    } else {
        println!(
            "PARITY: usearch R@10={:.3} < 0.95 ✗  — tune expansion_search",
            us_r10
        );
    }
    println!();

    // ── write dashboard doc ──────────────────────────────────────────────────
    let doc = format!(
        "# RAW ANN BENCH 2026-05-11\n\n\
         N={N} DIM={DIM} K={K} Q={QUERIES}  |  M4 Max  |  in-memory only\n\n\
         | Backend | p50 µs | p99 µs | R@10 |\n\
         |---------|--------|--------|------|\n\
         | FAISS-Flat (exact) | {flat_p50} | {flat_p99} | {flat_r10:.3} |\n\
         | FAISS-HNSW | {hnsw_p50} | {hnsw_p99} | {hnsw_r10:.3} |\n\
         | usearch-HNSW (synapse) | {us_p50} | {us_p99} | {us_r10:.3} |\n\n\
         usearch build: {build_ms}ms\n\n\
         Parity claim: usearch R@10={us_r10:.3} {} 0.95\n",
        if us_r10 >= 0.95 { "≥" } else { "<" }
    );
    std::fs::write(
        "/Users/master/projects/synapse/bench-dashboard/RAW_ANN_BENCH_2026-05-11.md",
        &doc,
    )
    .unwrap_or_else(|e| eprintln!("warn: could not write dashboard doc: {e}"));
    println!("Dashboard → bench-dashboard/RAW_ANN_BENCH_2026-05-11.md");
}

// ── npy helpers (minimal, no external dep) ───────────────────────────────────

#[cfg(feature = "ann-usearch")]
fn write_npy_f32_2d(path: &str, data: &[Vec<f32>], dim: usize) {
    use std::io::Write;
    let rows = data.len();
    // numpy header: shape=(rows, dim), dtype=float32, C order, little-endian
    let header = format!("{{'descr': '<f4', 'fortran_order': False, 'shape': ({rows}, {dim}), }}");
    // pad header to multiple of 64 (after magic+version+len = 10 bytes)
    let header_bytes = header.as_bytes();
    let prefix_len = 10usize; // magic(6)+ver(2)+len(2)
    let pad_len = (64 - (prefix_len + header_bytes.len() + 1) % 64) % 64;
    let mut hdr = header_bytes.to_vec();
    hdr.extend(std::iter::repeat(b' ').take(pad_len));
    hdr.push(b'\n');
    let hdr_len = hdr.len() as u16;

    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(b"\x93NUMPY\x01\x00").unwrap();
    f.write_all(&hdr_len.to_le_bytes()).unwrap();
    f.write_all(&hdr).unwrap();
    for row in data {
        for &x in row {
            f.write_all(&x.to_le_bytes()).unwrap();
        }
    }
}

#[cfg(feature = "ann-usearch")]
fn write_truths(path: &str, truths: &[Vec<u64>]) {
    use std::io::Write;
    let mut f = std::fs::File::create(path).unwrap();
    for row in truths {
        let line: Vec<String> = row.iter().map(|x| x.to_string()).collect();
        writeln!(f, "{}", line.join(" ")).unwrap();
    }
}

#[cfg(feature = "ann-usearch")]
fn write_faiss_script(path: &str, _n: usize, _dim: usize, k: usize, _queries: usize) {
    let script = format!(
        r#"#!/usr/bin/env python3
# auto-generated by raw_ann_microbench.rs
import numpy as np, time, sys, os

data   = np.load('/tmp/raw_ann_data.npy').astype('float32')
qvecs  = np.load('/tmp/raw_ann_queries.npy').astype('float32')
K      = {k}

with open('/tmp/raw_ann_truths.txt') as fh:
    truths = [[int(x) for x in l.split()] for l in fh if l.strip()]

try:
    import faiss
except ImportError:
    print("FAISS_UNAVAILABLE")
    sys.exit(0)

def recall(got_ids, truth):
    ts = set(truth)
    return sum(1 for x in got_ids if x in ts) / len(truth)

def pct(lats, p):
    return int(np.percentile(lats, p))

# ── FLAT (exact) ──────────────────────────────────────────────────────────────
flat = faiss.IndexFlatIP(data.shape[1])   # inner-product == cosine on L2-norm'd vecs
flat.add(data)

flat_lats, flat_recalls = [], []
for q, tr in zip(qvecs, truths):
    t0 = time.perf_counter()
    _, I = flat.search(q.reshape(1,-1), K)
    flat_lats.append((time.perf_counter()-t0)*1e6)
    flat_recalls.append(recall(I[0].tolist(), tr))

# ── HNSW ─────────────────────────────────────────────────────────────────────
hnsw = faiss.IndexHNSWFlat(data.shape[1], 16)   # M=16 matches usearch default
hnsw.hnsw.efSearch  = 256
hnsw.hnsw.efConstruction = 256
hnsw.add(data)

hnsw_lats, hnsw_recalls = [], []
for q, tr in zip(qvecs, truths):
    t0 = time.perf_counter()
    _, I = hnsw.search(q.reshape(1,-1), K)
    hnsw_lats.append((time.perf_counter()-t0)*1e6)
    hnsw_recalls.append(recall(I[0].tolist(), tr))

print(f"FLAT_P50={{pct(flat_lats,50)}} FLAT_P99={{pct(flat_lats,99)}} "
      f"HNSW_P50={{pct(hnsw_lats,50)}} HNSW_P99={{pct(hnsw_lats,99)}} "
      f"FLAT_R10={{sum(flat_recalls)/len(flat_recalls):.6f}} "
      f"HNSW_R10={{sum(hnsw_recalls)/len(hnsw_recalls):.6f}}")
"#
    );
    std::fs::write(path, script).unwrap();
}

#[cfg(feature = "ann-usearch")]
fn run_faiss_script(py_path: &str) -> (String, String, String, String, String, String) {
    let na = || "N/A".to_string();
    let out = match std::process::Command::new("python3").arg(py_path).output() {
        Ok(o) => o,
        Err(_) => return (na(), na(), na(), na(), na(), na()),
    };
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        eprintln!("faiss script stderr: {stderr}");
        return (na(), na(), na(), na(), na(), na());
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout
        .lines()
        .find(|l| l.contains("FLAT_P50") || l.contains("FAISS_UNAVAILABLE"));
    match line {
        None | Some("FAISS_UNAVAILABLE") => (na(), na(), na(), na(), na(), na()),
        Some(l) => {
            let kv = |key: &str| -> String {
                l.split_whitespace()
                    .find(|s| s.starts_with(key))
                    .and_then(|s| s.split('=').nth(1))
                    .unwrap_or("N/A")
                    .to_string()
            };
            (
                kv("FLAT_P50"),
                kv("FLAT_P99"),
                kv("FLAT_R10"),
                kv("HNSW_P50"),
                kv("HNSW_P99"),
                kv("HNSW_R10"),
            )
        }
    }
}
