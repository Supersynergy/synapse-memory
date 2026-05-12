# Stage 1: build
FROM rust:1.82-slim AS builder

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libsqlite3-dev \
    curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .

RUN cargo build --release --bin synapse --bin synapsed \
    && strip target/release/synapse \
    && strip target/release/synapsed

# Stage 2: runtime
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y \
    libssl3 \
    libsqlite3-0 \
    curl \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -r -s /bin/false -u 1000 synapse

COPY --from=builder /app/target/release/synapse /usr/local/bin/synapse
COPY --from=builder /app/target/release/synapsed /usr/local/bin/synapsed

RUN mkdir -p /data && chown synapse:synapse /data

USER synapse

ENV SYNAPSE_DB_PATH=/data/synapse.db \
    SYNAPSE_LOG_LEVEL=info \
    SYNAPSE_BIND=0.0.0.0:9477 \
    SYNAPSE_METRICS_BIND=0.0.0.0:9478

VOLUME ["/data"]

EXPOSE 9477 9478

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://127.0.0.1:9478/metrics || exit 1

ENTRYPOINT ["synapsed"]
