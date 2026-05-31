# Synapse Benchmark Kit

Reproducible proof kit for local agent memory.

## Product Promise

Run local benchmarks that show recall quality, latency, token savings, and
context-pack tradeoffs without trusting a marketing chart.

## Included

- Recall bakeoff against local memory candidates.
- AgentDB public benchmark and context-pack race.
- Three-session bugfix demo.
- Latest known result snapshots.

## Run From Monorepo

```bash
make bench-agent-memory
make demo-agent-memory
```

## Product Verify

```bash
products/benchmark-kit/scripts/verify.sh
```

## Current Best Result Snapshot

Latest recall bakeoff showed:

- `synapse_scoped_fusion_sdk`: R@5 `1.0`, MRR `1.0`, p50 `0.34ms`, p95 `0.48ms`.
- `synapse_mem0_compat_socket`: R@5 `1.0`, p95 `1.30ms`.
- `sqlite_fts5_control`: R@5 `0.625`.
- `claude_mem_worker_import_fts`: R@5 `0.0` in this small isolated gate.

This is a precision/latency gate, not a public SOTA claim.
