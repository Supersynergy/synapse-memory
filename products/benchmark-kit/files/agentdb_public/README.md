# Synapse AgentDB Public Benchmark

This benchmark verifies Synapse as a local database for coding agents.

It measures a scoped AgentDB workflow:

- typed observation ingest
- compact `search_index()` recall
- token-budgeted `context_pack()`
- progressive disclosure token savings
- a deterministic Autolearn-style race over context-pack configurations
- older target memories hidden behind a newer scoped filler tail

Run it with a local `synapsed` daemon:

```bash
PYTHONPATH=sdk/python python3 bench/agentdb_public/bench_agentdb_public.py
```

Outputs are written to `bench/agentdb_public/results/`.

Latest local verification:

```text
Run: ADB-20260516-122118
docs: 104 total = 8 targets + 96 scoped filler rows
search_index: R@1 0.9375 | R@5 1.0000 | MRR 0.9531 | p95 1.633 ms
best context arm: balanced
balanced: R@5 1.0000 | MRR 0.9531 | p95 0.858 ms | token savings 74.5% | avg hydrated 1
MCP agent_context smoke: schema synapse.agentdb.v1 | top_id correct | token savings 74.5% | truncated observation yes
```

The score used for picking the context-pack arm is intentionally simple:

```text
R@5*100 + MRR*20 + token_savings_pct*0.2 - p95_ms*0.5
```

This is a regression and tuning gate. It is not, by itself, a public SOTA
claim against LongMemEval, LoCoMo, BEAM, Mem0, Zep, Letta, Cognee, or
claude-mem.
