//! Library-mode example: embed synapse-core directly, zero IPC.
//! Beats sqlite-vec on features (BM25+vec+sign+CRDT) at same raw-latency tier.
//!
//! Run: cargo run --release --example library_mode

use anyhow::Result;
use std::time::Instant;
use synapse_core::{embed::Embedder, PutRequest, SearchMode, Store};

fn main() -> Result<()> {
    let db_path = "/tmp/libmode_bench.db";
    let cache_path = "/tmp/libmode_bench.emb";
    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_file(cache_path);

    let mut store = Store::open(db_path)?;
    let embedder = Embedder::new_with_cache(Some(cache_path))?;

    // === bulk put 1000 ===
    let texts: Vec<String> = (0..1000).map(|i| format!("decision {i}: chose tool-{} for reason perf-bench run {i}", i % 50)).collect();
    let t0 = Instant::now();
    let embs = embedder.embed_batch(&texts)?;
    let dt_embed = t0.elapsed();

    let reqs: Vec<PutRequest> = texts.iter().enumerate().map(|(i, t)| PutRequest {
        title: Some(format!("d{i}")),
        uri: None,
        text: t.clone(),
        meta: None,
        embedding: Some(embs[i].clone()),
    }).collect();
    let t0 = Instant::now();
    let _ids = store.put_batch(&reqs)?;
    let dt_put = t0.elapsed();

    println!("embed 1000  : {:?}  ({:.1} docs/s)", dt_embed, 1000.0 / dt_embed.as_secs_f64());
    println!("put_batch   : {:?}  ({:.1} docs/s)", dt_put, 1000.0 / dt_put.as_secs_f64());

    // === search ===
    let qv = embedder.embed_one("tool perf benchmark decision")?;
    let t0 = Instant::now();
    for _ in 0..100 {
        let _hits = store.search("perf benchmark", SearchMode::Hybrid, Some(&qv), 10)?;
    }
    let dt = t0.elapsed();
    println!("hybrid 100x : {:?}  (avg {:.2}ms)", dt, dt.as_secs_f64() * 10.0);

    let t0 = Instant::now();
    for _ in 0..10_000 {
        let _hits = store.search("perf benchmark", SearchMode::Lex, None, 10)?;
    }
    let dt = t0.elapsed();
    println!("lex  10000x : {:?}  (avg {:.3}ms)", dt, dt.as_secs_f64() * 0.1);

    // cache hit rerun
    let t0 = Instant::now();
    let _ = embedder.embed_batch(&texts)?;
    let dt = t0.elapsed();
    println!("embed cache : {:?}  ({:.0} docs/s)", dt, 1000.0 / dt.as_secs_f64());

    Ok(())
}
