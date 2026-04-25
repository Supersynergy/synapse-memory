//! Entry point for `synapse-mysql-async` binary (feature = "async-proxy").
//! Listens on :13310, proxies SELECT/ping via opensrv-mysql async shim.

mod server_async;

use opensrv_mysql::AsyncMysqlIntermediary;
use server_async::AsyncHandler;
use tokio::net::TcpListener;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let addr = "127.0.0.1:13310";
    let listener = TcpListener::bind(addr).await?;
    info!("synapse-mysql-async listening on {addr} (SELECT + ping only, Phase 1)");

    loop {
        let (stream, peer) = listener.accept().await?;
        info!("connection from {peer}");
        tokio::spawn(async move {
            let (r, w) = stream.into_split();
            if let Err(e) = AsyncMysqlIntermediary::run_on(AsyncHandler, r, w).await {
                tracing::warn!("handler error: {e}");
            }
        });
    }
}
