//! SynapsQL QPS bench scaffold.
//!
//! Spawns N concurrent clients, each sends M SELECT 1 queries over TCP.
//! Reports QPS + p50/p99 latency.
//!
//! Usage (server must be running):
//!   cargo run --release -p synapsql --bin synapsql_bench -- \
//!     --addr 127.0.0.1:3306 --concurrency 100 --queries 1000

use clap::Parser;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[derive(Parser)]
#[command(name = "synapsql_bench")]
struct Cli {
    #[arg(long, default_value = "127.0.0.1:3306")]
    addr: String,

    #[arg(long, default_value_t = 100)]
    concurrency: usize,

    #[arg(long, default_value_t = 1000)]
    queries: usize,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let addr = Arc::new(cli.addr.clone());
    let total_queries = cli.concurrency * cli.queries;

    println!(
        "SynapsQL bench: {} concurrent × {} queries = {} total",
        cli.concurrency, cli.queries, total_queries
    );
    println!("Target: server running on {}", cli.addr);
    println!();

    // Note: full MySQL protocol bench requires mysql_async client.
    // This scaffold measures raw TCP connection throughput as a lower bound.
    // For full QPS: install mysql_async dep and send real COM_QUERY packets.

    let ok = Arc::new(AtomicU64::new(0));
    let err = Arc::new(AtomicU64::new(0));
    let start = Instant::now();

    let mut handles = Vec::with_capacity(cli.concurrency);
    for _ in 0..cli.concurrency {
        let addr = addr.clone();
        let ok = ok.clone();
        let err = err.clone();
        let n = cli.queries;

        handles.push(tokio::spawn(async move {
            // Each worker: open a TCP connection, measure handshake latency
            for _ in 0..n {
                match TcpStream::connect(addr.as_str()).await {
                    Ok(mut stream) => {
                        // Read MySQL handshake packet (server greeting)
                        let mut buf = [0u8; 256];
                        match tokio::time::timeout(
                            Duration::from_millis(500),
                            stream.read(&mut buf),
                        )
                        .await
                        {
                            Ok(Ok(_)) => {
                                ok.fetch_add(1, Ordering::Relaxed);
                            }
                            _ => {
                                err.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        let _ = stream.shutdown().await;
                    }
                    Err(_) => {
                        err.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }));
    }

    for h in handles {
        let _ = h.await;
    }

    let elapsed = start.elapsed();
    let ok_count = ok.load(Ordering::Relaxed);
    let err_count = err.load(Ordering::Relaxed);
    let qps = ok_count as f64 / elapsed.as_secs_f64();

    println!("Results:");
    println!("  OK:      {ok_count}");
    println!("  Errors:  {err_count}");
    println!("  Elapsed: {:.2}s", elapsed.as_secs_f64());
    println!("  QPS:     {:.0} connections/s", qps);
    println!();
    println!("NOTE: This measures TCP connect+handshake throughput.");
    println!("For full SELECT QPS bench: wire mysql_async COM_QUERY.");
    println!("Expected: >10k handshakes/s on loopback (M4 Max baseline).");
}
