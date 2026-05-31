# Stage 1: build
FROM rust:1.95-slim AS builder

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    cmake \
    pkg-config \
    libssl-dev \
    libsqlite3-dev \
    curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY synapsestore ./synapsestore
COPY bench/longmemeval ./bench/longmemeval
COPY bench/space-vs-chroma ./bench/space-vs-chroma
COPY bench/industry ./bench/industry
COPY python/migrations ./python/migrations

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --bin synx --bin synapsed --bin synx-fast \
    && cp /app/target/release/synx /tmp/synx \
    && cp /app/target/release/synapsed /tmp/synapsed \
    && cp /app/target/release/synx-fast /tmp/synx-fast \
    && strip /tmp/synx \
    && strip /tmp/synapsed \
    && strip /tmp/synx-fast

# Stage 2: runtime
FROM debian:trixie-slim AS runtime

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl3t64 \
    libsqlite3-0 \
    curl \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -r -s /bin/false -u 1000 synapse

COPY --from=builder /tmp/synx /usr/local/bin/synx
COPY --from=builder /tmp/synapsed /usr/local/bin/synapsed
COPY --from=builder /tmp/synx-fast /usr/local/bin/synx-fast

RUN mkdir -p /data && chown synapse:synapse /data

USER synapse

ENV SYNAPSE_DB_PATH=/data/synapse.db \
    SYNAPSE_LOG_LEVEL=info \
    SYNAPSE_SOCK=/data/synapse.sock \
    SYNAPSE_METRICS_ADDR=0.0.0.0:9478

VOLUME ["/data"]

EXPOSE 9477 9478

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://127.0.0.1:9478/metrics || exit 1

ENTRYPOINT ["synapsed", "--file", "/data/synapse.db", "--sock", "/data/synapse.sock"]
