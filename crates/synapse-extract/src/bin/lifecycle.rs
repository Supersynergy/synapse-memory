use anyhow::Result;
use clap::Parser;
use rusqlite::Connection;
use synapse_core::sota_pipeline::{RuleHooks, compact};

#[derive(Parser, Debug)]
#[command(name = "synapse-lifecycle")]
struct Args {
    #[arg(long, default_value = "~/.synapse/brain.db")]
    db: String,

    #[arg(long, default_value_t = 0.7)]
    jaccard: f64,

    #[arg(long, default_value_t = 5000)]
    max_rows: usize,

    #[arg(long, default_value_t = 30.0)]
    decay_half_life_days: f64,
}

fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{}/{}", home, rest)
    } else {
        path.to_string()
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let db_path = expand_tilde(&args.db);

    tracing::info!(db = %db_path, jaccard = args.jaccard, max_rows = args.max_rows, "synapse-lifecycle starting");

    let conn = Connection::open(&db_path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;

    // Step 1: compact (cluster + supersede duplicates)
    let hooks = RuleHooks;
    let superseded = compact(&conn, &hooks, args.jaccard, args.max_rows)?;
    tracing::info!(superseded, "compact done");

    // Step 2: heat decay — weight *= 0.97 for memories not updated in 24h
    let decay_factor = 0.5f64.powf(1.0 / args.decay_half_life_days);
    let updated = conn.execute(
        "UPDATE memories SET weight = weight * ?1 WHERE updated_ts < unixepoch() - 86400",
        rusqlite::params![decay_factor],
    )?;
    tracing::info!(updated, decay_factor, "heat decay applied");

    // Step 3: vacuum
    conn.execute_batch("VACUUM;")?;
    tracing::info!("vacuum done");

    Ok(())
}
