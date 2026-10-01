//! HTTP/gRPC frontend — proxies :9477 (synapse-core turbo daemon).
//! TODO: full axum router wiring once synapse-core HTTP API is stabilised.

pub async fn serve(addr: &str) -> std::io::Result<()> {
    eprintln!("synapsql http: not implemented in v1.0.1-rc (addr={addr}); skipping");
    // Park forever so tokio::select! in Service::run doesn't exit on this arm.
    std::future::pending().await
}
