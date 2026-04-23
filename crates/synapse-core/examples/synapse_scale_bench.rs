//! Synapse scale-bench binary — PR-G2 of scale-100M plan.
//!
//! Apples-to-apples with bench/bench_scale_ladder.py — deterministic
//! sha256-derived 384d vectors, same N docs, same Q queries, top-k=10.
//! Reports ingest wall-time, query p50/p95/p99, disk footprint.
//!
//! Output: a single JSON line to stdout (the Python harness parses it).
//!
//! Invoke:
//!   ./target/release/examples/synapse_scale_bench --n 10000 --q 100
//!
//! Fair-play rules:
//!   * Vectors injected via PutRequest.embedding (no fastembed in the bench
//!     loop — the competitors also get pre-computed 384d vectors).
//!   * Single Db handle opened once; persistent SQLite file in /tmp.
//!   * Disk size captured via std::fs::metadata after commit.
//!   * No CRDT / signing / KG / federation paths — pure put+search.

use clap::Parser;
use sha2::{Digest, Sha256};
use std::time::Instant;
use synapse_core::db::Store;
use synapse_core::types::{PutRequest, SearchMode};

#[derive(Parser)]
#[command(about = "Synapse scale bench (PR-G2)")]
struct Opts {
    #[arg(long, default_value_t = 10_000)]
    n: usize,
    #[arg(long, default_value_t = 100)]
    q: usize,
    #[arg(long, default_value_t = 384)]
    dim: usize,
    /// Optional path for .synapse/brain.db; if omitted, uses a mktemp dir.
    #[arg(long)]
    file: Option<String>,
}

fn sha_vec(seed: &str, dim: usize) -> Vec<f32> {
    let mut h = Sha256::new();
    h.update(seed.as_bytes());
    let digest = h.finalize();
    let mut out = Vec::with_capacity(dim);
    for i in 0..dim {
        let start = (i * 4) % digest.len();
        let end = start + 4;
        let chunk = if end <= digest.len() {
            &digest[start..end]
        } else {
            &digest[start..]
        };
        let mut buf = [0u8; 4];
        let n = chunk.len().min(4);
        buf[..n].copy_from_slice(&chunk[..n]);
        let raw = u32::from_le_bytes(buf);
        let v = (raw as f64) / 2f64.powi(31) - 1.0;
        out.push(v as f32);
    }
    out
}

fn pct(xs: &mut [f64], p: f64) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let k = ((xs.len() as f64 - 1.0) * p / 100.0).round() as usize;
    xs[k.min(xs.len() - 1)]
}

fn dir_size(p: &std::path::Path) -> u64 {
    let mut total = 0;
    if let Ok(md) = std::fs::metadata(p) {
        if md.is_file() {
            return md.len();
        }
    }
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            if let Ok(md) = e.metadata() {
                total += if md.is_dir() {
                    dir_size(&e.path())
                } else {
                    md.len()
                };
            }
        }
    }
    total
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let o = Opts::parse();

    // Fresh DB file in tmp
    let tmp = tempfile::tempdir()?;
    let path = o
        .file
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| tmp.path().join("brain.db"));
    let mut db = Store::open(&path)?;

    // Build deterministic corpus
    let t_build = Instant::now();
    let reqs: Vec<PutRequest> = (0..o.n)
        .map(|i| {
            let text = format!("doc {i} topic{}", i % 37);
            PutRequest {
                text: text.clone(),
                embedding: Some(sha_vec(&text, o.dim)),
                ..Default::default()
            }
        })
        .collect();
    let build_ms = t_build.elapsed().as_secs_f64() * 1000.0;

    // Ingest via put_batch in 5k chunks (same BATCH as Python harness)
    let t_ing = Instant::now();
    const BATCH: usize = 5_000;
    for chunk in reqs.chunks(BATCH) {
        db.put_batch(chunk)?;
    }
    let ingest_s = t_ing.elapsed().as_secs_f64();

    // Queries — use the same deterministic sha vectors offset-by-5 matching Python harness
    let queries: Vec<Vec<f32>> = (0..o.q)
        .map(|i| sha_vec(&format!("doc {} topic{}", i * 5, (i * 5) % 37), o.dim))
        .collect();

    let mut lat_ms: Vec<f64> = Vec::with_capacity(o.q);
    for qv in &queries {
        let t = Instant::now();
        let _ = db.search("", SearchMode::Vec, Some(qv), 10)?;
        lat_ms.push(t.elapsed().as_secs_f64() * 1000.0);
    }

    let p50 = pct(&mut lat_ms.clone(), 50.0);
    let p95 = pct(&mut lat_ms.clone(), 95.0);
    let p99 = pct(&mut lat_ms.clone(), 99.0);
    let mean: f64 = lat_ms.iter().sum::<f64>() / (lat_ms.len() as f64);

    drop(db);
    let disk_bytes = dir_size(&path);

    let out = serde_json::json!({
        "engine": "Synapse v2",
        "N": o.n,
        "dim": o.dim,
        "Q": o.q,
        "build_ms": build_ms,
        "ingest_s": ingest_s,
        "query_p50_ms": p50,
        "query_p95_ms": p95,
        "query_p99_ms": p99,
        "query_mean_ms": mean,
        "disk_bytes": disk_bytes,
        "disk_mb": disk_bytes as f64 / 1_000_000.0,
        "ok": true,
    });
    println!("{out}");
    Ok(())
}
