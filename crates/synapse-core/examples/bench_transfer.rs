//! Synapse "transfer" bench — simulates 1M financial transfers as doc inserts.
//!
//! Each transfer = 1 PutRequest with JSON payload (debit+credit fields).
//! Uses put_batch_fast (synchronous=OFF, single WAL tx per batch).
//!
//! Run: cargo run --example bench_transfer --release
//!      (no extra features needed)

use std::time::Instant;
use synapse_core::{PutRequest, Store};

const TOTAL: usize = 1_000_000;
const BATCH: usize = 8_192;

fn main() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let mut s = Store::open(tmp.path()).unwrap();

    let mut total_inserted: usize = 0;
    let mut batch_latencies_us: Vec<u64> = Vec::with_capacity(TOTAL / BATCH + 1);

    eprintln!("Synapse bench_transfer: {TOTAL} transfers, batch={BATCH}");

    let t_global = Instant::now();

    let mut offset = 0usize;
    while offset < TOTAL {
        let end = (offset + BATCH).min(TOTAL);
        let reqs: Vec<PutRequest> = (offset..end)
            .map(|i| PutRequest {
                // Mimic a debit+credit transfer record
                text: format!(
                    r#"{{"id":{},"from":{},"to":{},"amount":{},"currency":"EUR","ts":{}}}"#,
                    i,
                    i % 10_000,
                    (i + 1) % 10_000,
                    (i % 10_000) + 1,
                    1_747_000_000u64 + i as u64
                ),
                title: Some(format!("tx-{i}")),
                embedding: None,
                uri: Some(format!("transfer:{i}")),
                ..Default::default()
            })
            .collect();

        let t0 = Instant::now();
        let ids = s.put_batch_fast(&reqs).unwrap();
        let elapsed_us = t0.elapsed().as_micros() as u64;

        total_inserted += ids.len();
        batch_latencies_us.push(elapsed_us);
        offset = end;
    }

    let total_elapsed = t_global.elapsed();
    let tps = total_inserted as f64 / total_elapsed.as_secs_f64();

    // percentiles
    batch_latencies_us.sort_unstable();
    let n = batch_latencies_us.len();
    let p50 = batch_latencies_us[n / 2];
    let p99 = batch_latencies_us[(n as f64 * 0.99) as usize];
    let p100 = *batch_latencies_us.last().unwrap();

    eprintln!();
    eprintln!("=== Synapse bench_transfer results ===");
    eprintln!("total transfers : {total_inserted}");
    eprintln!("total time      : {total_elapsed:.2?}");
    eprintln!("throughput      : {tps:.0} tx/s");
    eprintln!("batch latency p50  : {} ms", p50 / 1000);
    eprintln!("batch latency p99  : {} ms", p99 / 1000);
    eprintln!("batch latency p100 : {} ms", p100 / 1000);
    eprintln!("(batch={BATCH} transfers/tx, synchronous=OFF, WAL, no-embed, FTS5 dedup hash)");
}
