# synapse-server — production Docker image
# Multi-stage: builder (rust toolchain) → runtime (distroless ~5MB final + bin)

FROM rust:1.95-slim AS builder
WORKDIR /build
RUN apt-get update && apt-get install -y pkg-config libssl-dev cmake clang && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo build --release -p synapse-server

FROM gcr.io/distroless/cc-debian12
COPY --from=builder /build/target/release/synapse-server /usr/local/bin/synapse-server
EXPOSE 3306 5432 9990
VOLUME ["/data"]
ENV RUST_LOG=info
ENTRYPOINT ["/usr/local/bin/synapse-server"]
CMD ["--mysql", "0.0.0.0:3306", \
     "--pg", "0.0.0.0:5432", \
     "--ops-http", "0.0.0.0:9990", \
     "--db", "/data/synapse.db", \
     "--backend", "pool", \
     "--pool-size", "16", \
     "--turbo", \
     "--autolearn", \
     "--slow-threshold-ms", "100"]
