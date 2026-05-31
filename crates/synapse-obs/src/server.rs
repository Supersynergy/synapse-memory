//! Tokio task: axum HTTP server exposing Prometheus metrics at :9478/metrics.

use axum::{Router, routing::get};
use prometheus::Encoder as _;

pub async fn spawn_metrics_server() {
    tokio::spawn(async {
        let app = Router::new().route("/metrics", get(metrics_handler));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:9478")
            .await
            .expect("bind :9478 for metrics");
        axum::serve(listener, app)
            .await
            .expect("metrics server exited");
    });
}

async fn metrics_handler() -> impl axum::response::IntoResponse {
    let registry = crate::metrics::registry();
    let encoder = prometheus::TextEncoder::new();
    let families = registry.gather();
    let mut buf = Vec::with_capacity(4096);
    encoder.encode(&families, &mut buf).unwrap_or_default();
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        buf,
    )
}
