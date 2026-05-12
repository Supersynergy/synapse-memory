//! OpenTelemetry tracer init — OTLP gRPC exporter to localhost:4317.

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{runtime, trace as sdktrace};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub fn init_tracer() -> anyhow::Result<()> {
    let exporter = opentelemetry_otlp::new_exporter()
        .tonic()
        .with_endpoint("http://localhost:4317");

    let tracer_provider = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(exporter)
        .with_trace_config(sdktrace::Config::default().with_resource(
            opentelemetry_sdk::Resource::new(vec![
                opentelemetry::KeyValue::new("service.name", "synapse"),
                opentelemetry::KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
            ]),
        ))
        .install_batch(runtime::Tokio)?;

    let tracer = tracer_provider.tracer("synapse-core");
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer().json())
        .with(otel_layer)
        .try_init()
        .ok(); // ignore if already initialized

    Ok(())
}

/// Flush all pending spans. Call on graceful shutdown.
pub fn shutdown() {
    opentelemetry::global::shutdown_tracer_provider();
}
