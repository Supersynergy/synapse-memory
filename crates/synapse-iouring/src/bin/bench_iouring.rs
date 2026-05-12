//! bench_iouring — Linux io_uring vs rusqlite WAL 100k insert bench.
//!
//! Run on Linux with: cargo run --bin bench_iouring --features io-uring --release
//! On macOS: no-op (prints "io-uring feature required").

#[cfg(feature = "io-uring")]
mod bench {
    use std::time::Instant;
    use synapse_iouring::{Entry, IoUringStore};

    pub async fn run() {
        let n: usize = 100_000;
        println!("=== synapse-iouring Linux bench ({n} inserts) ===\n");

        // ── io_uring WAL ──────────────────────────────────────────────────────
        // Use /tmp (ext4/tmpfs) not virtiofs-mounted paths for io_uring writes
        let tmp = std::path::PathBuf::from("/tmp/synapse_iouring_bench");
        std::fs::create_dir_all(&tmp).unwrap();
        let mut store = IoUringStore::open(&tmp).expect("open store");

        let batch_size = 512;
        let t0 = Instant::now();
        for chunk_start in (0..n).step_by(batch_size) {
            let end = (chunk_start + batch_size).min(n);
            let entries: Vec<Entry> = (chunk_start..end)
                .map(|i| Entry {
                    key: format!("key{:08}", i).into_bytes(),
                    value: format!("value_{}", i).into_bytes(),
                    seq: i as u64,
                    deleted: false,
                })
                .collect();
            store.append_batch(entries).await.expect("append");
        }
        let uring_ms = t0.elapsed().as_millis();
        let uring_per_s = n as f64 / t0.elapsed().as_secs_f64();

        println!("io_uring WAL:");
        println!("  total : {}ms", uring_ms);
        println!("  rate  : {:.0} inserts/s", uring_per_s);

        // ── rusqlite WAL baseline ─────────────────────────────────────────────
        let sqlite_tmp = std::path::PathBuf::from("/tmp/synapse_sqlite_bench");
        std::fs::create_dir_all(&sqlite_tmp).unwrap();
        let db_path = sqlite_tmp.join("bench.db");
        let conn = rusqlite::Connection::open(&db_path).expect("sqlite open");
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
             CREATE TABLE kv (key TEXT PRIMARY KEY, value TEXT, seq INTEGER);",
        )
        .unwrap();

        let t1 = Instant::now();
        for chunk_start in (0..n).step_by(batch_size) {
            let end = (chunk_start + batch_size).min(n);
            let tx = conn.unchecked_transaction().unwrap();
            for i in chunk_start..end {
                tx.execute(
                    "INSERT OR REPLACE INTO kv VALUES (?1, ?2, ?3)",
                    rusqlite::params![
                        format!("key{:08}", i),
                        format!("value_{}", i),
                        i as i64
                    ],
                )
                .unwrap();
            }
            tx.commit().unwrap();
        }
        let sqlite_ms = t1.elapsed().as_millis();
        let sqlite_per_s = n as f64 / t1.elapsed().as_secs_f64();

        println!("\nrusqlite WAL (baseline):");
        println!("  total : {}ms", sqlite_ms);
        println!("  rate  : {:.0} inserts/s", sqlite_per_s);

        println!("\n=== SPEEDUP ===");
        let speedup = sqlite_ms as f64 / uring_ms as f64;
        println!("io_uring vs SQLite: {:.2}× faster", speedup);

        // Write markdown result
        let md = format!(
            r#"# io_uring Linux Bench — 2026-05-13

## Config
- n = {n} inserts, batch_size = {batch_size}
- OS: Linux aarch64 (colima Docker Ubuntu 24.04)
- Rust release build, --features io-uring

## Results

| Engine | Total (ms) | Inserts/s |
|--------|-----------|-----------|
| io_uring WAL | {uring_ms} | {uring_per_s:.0} |
| rusqlite WAL | {sqlite_ms} | {sqlite_per_s:.0} |

**Speedup: {speedup:.2}×**

## Notes
- io_uring batch_size=512 SQEs per submit
- SQLite: WAL mode, synchronous=NORMAL, batch transactions
"#
        );
        std::fs::write("/synapse/bench-dashboard/IOURING_LINUX_BENCH_2026-05-13.md", &md)
            .unwrap_or_else(|_| {
                // fallback if volume not mounted
                println!("\n--- MARKDOWN ---\n{}", md);
            });
        println!("Done.");
    }
}

#[tokio::main]
async fn main() {
    #[cfg(feature = "io-uring")]
    bench::run().await;

    #[cfg(not(feature = "io-uring"))]
    eprintln!("io-uring feature not enabled — run with --features io-uring on Linux");
}
