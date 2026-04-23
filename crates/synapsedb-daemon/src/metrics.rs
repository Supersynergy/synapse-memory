//! Prometheus metrics server on a separate TCP port.
//! Config: SYNAPSE_METRICS_ADDR (default 127.0.0.1:9090)

use anyhow::Result;
use axum::{routing::get, Router};
use metrics::{counter, gauge, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::net::SocketAddr;
use std::time::Duration;

pub struct MetricsHandle {
    pub handle: PrometheusHandle,
}

impl MetricsHandle {
    pub fn install() -> Result<Self> {
        let handle = PrometheusBuilder::new()
            .install_recorder()
            .map_err(|e| anyhow::anyhow!("metrics recorder: {e}"))?;
        Ok(Self { handle })
    }

    pub fn render(&self) -> String {
        self.handle.render()
    }
}

pub async fn serve(handle: PrometheusHandle, addr: SocketAddr) {
    let app = Router::new().route(
        "/metrics",
        get(move || {
            let h = handle.clone();
            async move { h.render() }
        }),
    );
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("metrics bind {addr}: {e}");
            return;
        }
    };
    tracing::info!("metrics on http://{addr}/metrics");
    if let Err(e) = axum::serve(listener, app).await {
        tracing::error!("metrics server: {e}");
    }
}

// ── Helpers called from main dispatch ────────────────────────────────────────

pub fn record_put(duration: Duration) {
    counter!("synapse_put_total").increment(1);
    histogram!("synapse_put_duration_seconds").record(duration.as_secs_f64());
}

pub fn record_query(mode: &str, duration: Duration) {
    counter!("synapse_query_total").increment(1);
    histogram!("synapse_query_duration_seconds", "mode" => mode.to_string())
        .record(duration.as_secs_f64());
}

pub fn record_embed(duration: Duration) {
    histogram!("synapse_embed_duration_seconds").record(duration.as_secs_f64());
}

pub fn record_shard_hit(shard: &str) {
    counter!("synapse_shard_hit_total", "shard" => shard.to_string()).increment(1);
}

pub fn set_doc_count(n: i64) {
    gauge!("synapse_doc_count").set(n as f64);
}

pub fn set_shard_count(n: usize) {
    gauge!("synapse_shard_count").set(n as f64);
}

pub fn set_queue_depth(n: usize) {
    gauge!("synapse_queue_depth").set(n as f64);
}
