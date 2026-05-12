//! HTTP/gRPC frontend — proxies :9477 (synapse-core turbo daemon).
//! TODO: full axum router wiring once synapse-core HTTP API is stabilised.

use tokio::net::TcpListener;

pub async fn serve(addr: &str) -> std::io::Result<()> {
    let _listener = TcpListener::bind(addr).await?;
    eprintln!("synapsql-http scaffold listening on {} (TODO: axum router)", addr);
    // Park forever; real impl will delegate to synapse-core::server.
    std::future::pending::<std::io::Result<()>>().await
}
