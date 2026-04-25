//! Entry point for `synapse-mysql-async` binary (feature = "async-proxy").
//! Full SELECT/INSERT/UPDATE/DELETE handler — Phase 2.

mod rewrite;
mod server_async;

use clap::Parser;
use opensrv_mysql::AsyncMysqlIntermediary;
use server_async::{AsyncHandler, SharedDb};
use std::path::PathBuf;
use tokio::net::TcpListener;
use tracing::info;

#[derive(Parser)]
#[command(name = "synapse-mysql-async")]
struct Cli {
    #[arg(long, default_value = "127.0.0.1:13310")]
    bind: String,

    #[arg(long, default_value = "/tmp/sync_test.db")]
    db: PathBuf,

    #[arg(long, default_value = "wp")]
    mode: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    let shared = SharedDb::new(cli.db, cli.mode)?;
    let listener = TcpListener::bind(&cli.bind).await?;
    info!("synapse-mysql-async listening on {} (Phase 2 — full handler)", cli.bind);

    loop {
        let (stream, peer) = listener.accept().await?;
        info!("connection from {peer}");
        let shared = shared.clone();
        tokio::spawn(async move {
            let handler = match AsyncHandler::new(shared) {
                Ok(h) => h,
                Err(e) => {
                    tracing::warn!("handler init error: {e}");
                    return;
                }
            };
            let (r, w) = stream.into_split();
            if let Err(e) = AsyncMysqlIntermediary::run_on(handler, r, w).await {
                tracing::warn!("handler error: {e}");
            }
        });
    }
}
