use anyhow::Result;
use clap::Parser;
use rusqlite::Connection;
use std::time::Duration;
use synapse_extract::{run_once, RuleExtractor};
#[allow(unused_imports)]
use tracing_subscriber;

#[cfg(feature = "minimax")]
use synapse_extract::minimax::MinimaxExtractor;

#[derive(Parser, Debug)]
#[command(name = "synapse-extract-worker")]
struct Args {
    #[arg(long, default_value = "~/.synapse/brain.db")]
    db: String,

    #[arg(long, default_value_t = 16)]
    batch: usize,

    #[arg(long, default_value_t = 30_000)]
    interval_ms: u64,

    #[arg(long, default_value = "auto")]
    extractor: String,

    #[arg(long)]
    once: bool,
}

fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{}/{}", home, rest)
    } else {
        path.to_string()
    }
}

fn open_db(path: &str) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    Ok(conn)
}

fn resolve_extractor(arg: &str) -> &'static str {
    match arg {
        "rule" => "rule",
        "mlx" => "mlx",
        "minimax" => "minimax",
        _ => {
            if std::env::var("MINIMAX_API_KEY")
                .map(|v| !v.is_empty())
                .unwrap_or(false)
            {
                "minimax"
            } else {
                "rule"
            }
        }
    }
}

fn run_extraction(conn: &Connection, extractor_name: &str, batch: usize) -> Result<usize> {
    match extractor_name {
        #[cfg(feature = "minimax")]
        "minimax" => {
            let ext = MinimaxExtractor::from_env()?;
            run_once(conn, &ext, batch)
        }
        _ => run_once(conn, &RuleExtractor, batch),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let db_path = expand_tilde(&args.db);
    let extractor_name = resolve_extractor(&args.extractor);
    let interval = Duration::from_millis(args.interval_ms);

    tracing::info!(db = %db_path, extractor = extractor_name, batch = args.batch, "synapse-extract-worker starting");

    if args.once {
        let conn = open_db(&db_path)?;
        let n = run_extraction(&conn, extractor_name, args.batch)?;
        tracing::info!(produced = n, "run_once complete");
        return Ok(());
    }

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("received SIGINT, shutting down");
                break;
            }
            _ = tokio::time::sleep(Duration::from_millis(0)) => {
                let conn = open_db(&db_path)?;
                match run_extraction(&conn, extractor_name, args.batch) {
                    Ok(n) => tracing::info!(produced = n, "extraction pass done"),
                    Err(e) => tracing::error!(error = %e, "extraction pass failed"),
                }
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {
                        tracing::info!("received SIGINT, shutting down");
                        break;
                    }
                    _ = tokio::time::sleep(interval) => {}
                }
            }
        }
    }

    Ok(())
}
