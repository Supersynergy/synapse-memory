# Pingora vs Axum — REAL apples-to-apples (2026-05-06)

Both Rust, both `/health` returns "ok", both M4 Max, oha c=256 z=20s.

## Setup
- **axum 0.8.9** standalone bin `axum_health` on `127.0.0.1:9479`
- **pingora 0.8.0** synapse-edge bin on `127.0.0.1:9478` (request_filter short-circuit, no proxy hop on /health)
- 16 worker threads each
- LTO=fat, opt-level=3, codegen-units=1

## Results

| Metric | axum 0.8 :9479 | pingora 0.8 :9478 | Winner |
|--------|----------------|---------------------|--------|
| QPS | **104 586** | 75 642 | axum +38% |
| p50 | 2.40 ms | 3.41 ms | axum -30% |
| p90 | 2.78 ms | 4.01 ms | axum |
| p99 | 5.86 ms | 5.47 ms | pingora -7% |
| **p99.9** | 21.07 ms | **8.97 ms** | **pingora -57%** |
| p99.99 | 48.13 ms | 35.37 ms | pingora -27% |
| Success | 100% | 100% | tie |
| Total req (20s) | 2 092 887 | 1 515 061 | axum |

## Truth-check vs prior bench

Prior bench (Python http.server upstream) showed pingora 7.4× over upstream — that was **Python GIL-bound mock**, not apples-to-apples. Real Rust-vs-Rust reverses it: axum wins QPS by 38%.

## Verdict

**Use axum for synapsed.** Pingora overhead = single proxy hop costs ~30% throughput on simple GETs.

**Pingora justified ONLY for:**
- Multi-upstream load-balancing (sharded synapse cluster)
- TLS termination separated from app
- Graceful binary upgrades (zero-downtime)
- Worker-process isolation
- Tail-latency-critical workloads (p99.9 wins by -57%)

**Recommendation:** Keep `synapse-edge` crate as optional edge-proxy for cluster-mode deployments. Default single-node = axum direct.
