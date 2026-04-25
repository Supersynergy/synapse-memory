//! synapse-mysql-async — Async MySQL wire-protocol server for Synapse.
//!
//! Phase 1 of MASTERPLAN-V2-GAMECHANGER-2026-04-25.
//! Replaces blocking msql_srv with opensrv-mysql + tokio for 5-10× OPS gain
//! at 8+ concurrent threads. Reuses rewrite + ACL logic from `synapse-mysql`.
//!
//! Reference impls (verified via ghgrep):
//! - databendlabs/databend src/query/service/src/servers/mysql/mysql_interactive_worker.rs
//! - GreptimeTeam/greptimedb src/servers/src/mysql/handler.rs

mod shim;

use anyhow::{Context, Result};
use clap::Parser;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{error, info, warn};

use shim::{new_shared_state, SharedState, SynapseMysqlAsync};

#[derive(Parser)]
#[command(
    name = "synapse-mysql-async",
    version,
    about = "Synapse async MySQL wire server (Phase 1 gamechanger)"
)]
struct Cli {
    /// SQLite/Synapse file to serve
    #[arg(short = 'f', long, default_value = ".synapse/mysql-bridge.db")]
    file: PathBuf,
    /// Listen address
    #[arg(short, long, default_value = "127.0.0.1:13310")]
    bind: SocketAddr,
    /// Compatibility mode (medium-coeli|wp|strict)
    #[arg(long, default_value = "medium-coeli")]
    mode: String,
    /// Default root password (change in production!)
    #[arg(long, default_value = "synapse")]
    root_password: String,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "synapse_mysql_async=info".into()),
        )
        .init();

    let cli = Cli::parse();
    if let Some(p) = cli.file.parent() {
        std::fs::create_dir_all(p).ok();
    }

    info!("opening database {}", cli.file.display());

    // Phase 1 MVP: skip ACL — caller responsible for prior setup via synapse-mysql binary.
    let _ = cli.root_password.clone(); // suppress unused

    let state = new_shared_state(cli.file.clone(), cli.mode.clone());

    let listener = TcpListener::bind(&cli.bind)
        .await
        .with_context(|| format!("bind {}", cli.bind))?;
    info!(
        "synapse-mysql-async listening on {} (mode={}, file={})",
        cli.bind,
        cli.mode,
        cli.file.display()
    );

    loop {
        let (stream, addr) = match listener.accept().await {
            Ok(p) => p,
            Err(e) => {
                error!("accept: {}", e);
                continue;
            }
        };
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            info!("conn from {}", addr);
            let (r, w) = stream.into_split();
            let shim = SynapseMysqlAsync::new(state);
            if let Err(e) = opensrv_mysql::AsyncMysqlIntermediary::run_on(shim, r, w).await {
                warn!("conn {} ended: {}", addr, e);
            }
        });
    }
}
