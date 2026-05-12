//! synapse-obs — production observability layer.
//!
//! Feature-gated behind `observability` — zero overhead when off.
//!
//! When enabled:
//! - OpenTelemetry traces exported via OTLP gRPC → localhost:4317
//! - Prometheus metrics endpoint on :9478/metrics
//! - `#[tracing::instrument]` on core hot-paths in synapse-core
//!
//! # Usage
//! ```ignore
//! // In your binary (feature = "observability"):
//! synapse_obs::init().await?;
//! // Metrics served automatically at :9478/metrics
//! ```

#[cfg(feature = "observability")]
pub mod metrics;
#[cfg(feature = "observability")]
pub mod otel;
#[cfg(feature = "observability")]
pub mod server;

#[cfg(feature = "observability")]
pub use metrics::Metrics;

/// Initialize the full observability stack: OTLP exporter + Prometheus endpoint.
///
/// Call once at binary startup before any queries. Spawns a background Tokio
/// task to serve `:9478/metrics`.
#[cfg(feature = "observability")]
pub async fn init() -> anyhow::Result<()> {
    otel::init_tracer()?;
    server::spawn_metrics_server().await;
    Ok(())
}

/// No-op when feature is off.
#[cfg(not(feature = "observability"))]
pub async fn init() -> Result<(), std::convert::Infallible> {
    Ok(())
}
