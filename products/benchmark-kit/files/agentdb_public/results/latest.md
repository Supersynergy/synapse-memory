# Synapse AgentDB Public Benchmark

Run: `ADB-20260516-122118`
Project: `agentdb-public-ADB-20260516-122118`
Stored docs: `104` (targets `8`, filler `96`); queries: `16`; top-k: `5`
Ingest: `1225.151 ms`

## Search Index

| R@1 | R@5 | MRR | p50 ms | p95 ms | max ms |
|---:|---:|---:|---:|---:|---:|
| 0.9375 | 1.0 | 0.9531 | 0.737 | 1.633 | 2.057 |

## Context-Pack Race

| Rank | Arm | R@5 | MRR | p95 ms | Token savings | Avg hydrated | Score |
|---:|---|---:|---:|---:|---:|---:|---:|
| 1 | `balanced` | 1.0 | 0.9531 | 0.858 | 74.5% | 1 | 133.533 |
| 2 | `speed` | 1.0 | 0.9531 | 0.886 | 74.1% | 1 | 133.433 |
| 3 | `token_saver` | 1.0 | 0.9531 | 156.983 | 79.1% | 1 | 56.39 |
| 4 | `recall` | 1.0 | 0.9531 | 168.571 | 48.9% | 3 | 44.557 |

## Winner

`balanced` with config `{"full_k": 2, "index_k": 8, "snippet_chars": 220, "token_budget": 1100}`.

Score is a local tuning objective: `R@5*100 + MRR*20 + token_savings*0.2 - p95_ms*0.5`.
It is useful for regression and config selection, not a public SOTA recall claim by itself.
