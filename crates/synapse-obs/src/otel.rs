//! OpenTelemetry tracer init — OTLP gRPC exporter to localhost:4317.

use opentelemetry::global;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{Resource, runtime, trace as sdktrace};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

pub fn init_tracer() -> anyhow::Result<()> {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint("http://localhost:4317")
        .build()?;

    let tracer_provider = sdktrace::TracerProvider::builder()
        .with_batch_exporter(exporter, runtime::Tokio)
        .with_resource(Resource::new(vec![
            opentelemetry::KeyValue::new("service.name", "synapse"),
            opentelemetry::KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
        ]))
        .build();

    let tracer = tracer_provider.tracer("synapse-core");
    global::set_tracer_provider(tracer_provider);
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
