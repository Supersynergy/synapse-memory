//! SynapsQL multi-protocol server binary.
//!
//! Usage:
//!   synapsql start --mysql 127.0.0.1:3306 --db ./test.synx
//!   synapsql start                          # all defaults
//!   synapsql version
//!
//! KILLER NUMBERS (target):
//!   ≥10k QPS single core   (SELECT 1, warm cache)
//!   ≥100k QPS multi-core   (tokio-task-per-conn, shared-nothing hot path)
//!   ≤50µs p99 latency      (local TCP loopback)

use clap::{Parser, Subcommand};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use synapsql::Service;

#[derive(Parser)]
#[command(name = "synapsql", about = "SynapsQL — MySQL/PG-wire + Vector + FTS. One binary. Zero config.")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,

    #[arg(long, default_value = "127.0.0.1:3306", global = true)]
    mysql: String,

    #[arg(long, default_value = "127.0.0.1:5432", global = true)]
    pg: String,

    #[arg(long, default_value = "127.0.0.1:9477", global = true)]
    http: String,

    #[arg(long, default_value = "./synapsql.synx", global = true)]
    db: String,
}

#[derive(Subcommand)]
enum Cmd {
    /// Start the server (default if no subcommand).
    Start,
    /// Print version info.
    Version,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("synapsql=info".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();

    match cli.cmd {
        Some(Cmd::Version) | None if false => {}
        Some(Cmd::Version) => {
            println!("SynapsQL {}", env!("CARGO_PKG_VERSION"));
            println!("  MySQL wire    {}", cli.mysql);
            println!("  Postgres wire {}", cli.pg);
            println!("  HTTP/turbo    {}", cli.http);
            println!("  Vec `<=>`     HNSW rewrite");
            println!("  HYBRID_RANK   RRF fusion");
            return Ok(());
        }
        _ => {}
    }

    let store: Arc<dyn synapse_libsql::Store> = Arc::new(SynapsStore::new());

    // Spawn QPS reporter (logs every 5s)
    tokio::spawn(qps_reporter());

    tracing::info!("SynapsQL starting — mysql={} pg={} http={}", cli.mysql, cli.pg, cli.http);

    Service::new(store)
        .with_mysql(&cli.mysql)
        .with_pg(&cli.pg)
        .with_http(&cli.http)
        .run()
        .await
}

/// Log QPS every 5 seconds.
async fn qps_reporter() {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        interval.tick().await;
        let q = synapse_mysql::drain_qps();
        if q > 0 {
            tracing::info!("QPS[5s avg]: {}/s", q / 5);
        }
    }
}

/// Production SynapsStore wrapping future synapse-core::Brain.
/// TODO: replace with Brain::open(path) once synapse-core API is wired.
struct SynapsStore {
    queries: AtomicU64,
}

impl SynapsStore {
    fn new() -> Self { Self { queries: AtomicU64::new(0) } }
}

#[async_trait::async_trait]
impl synapse_libsql::Store for SynapsStore {
    async fn query(&self, sql: &str) -> Result<synapse_libsql::QueryResult, synapse_libsql::Error> {
        self.queries.fetch_add(1, Ordering::Relaxed);
        tracing::debug!("query: {sql}");
        Ok(synapse_libsql::QueryResult::default())
    }
    async fn exec(&self, sql: &str) -> Result<u64, synapse_libsql::Error> {
        tracing::debug!("exec: {sql}");
        Ok(0)
    }
}
