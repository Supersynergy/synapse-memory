# synapse-edge

Pingora HTTP frontend for Synapse — **additive only**, zero changes to `synapsed`.

## Purpose
Benchmark Pingora vs Axum under load. Routes proxy to `synapsed` on `:9477`.

## Routes
- `GET /health` → upstream :9477 (502 if down — edge listener stays alive)
- `POST /embed /search /hybrid` → upstream :9477

## Run
```bash
cargo build -p synapse-edge --release
SYNAPSE_EDGE_PORT=9478 ./target/release/synapse-edge
```

## Bench
```bash
# Start synapsed first, then synapse-edge, then:
bash bench/scripts/2026-05-06/bench_pingora_vs_axum.sh
```
Results → `bench/results/2026-05-06/pingora-vs-axum.md`
