# Recall Bakeoff

Small local benchmark for Synapse vs installed memory/context candidates.

It uses unique per-run facts, ingests them into each available engine, runs the
same query set, and writes JSON plus Markdown reports under `results/`.

```bash
python3 bench/recall_bakeoff/run.py
python3 bench/recall_bakeoff/run.py --engines synapse,synapse_scoped --run-id RBK-DEBUG
python3 bench/recall_bakeoff/run.py --engines sqlite,synapse,synapse_scoped,synapse_mem0,claude_mem --run-id RBK-CORE
```

Scope:

- Synapse hybrid socket
- Synapse scoped lexical+hybrid fusion prototype
- Synapse mem0-compatible socket facade
- OMEGA CLI
- Signet daemon CLI
- Mem0 local FAISS smoke path
- Cognee session recall
- Letta archive passages API
- claude-mem isolated worker import/search path
- SQLite FTS5 lexical control

Latest verified full local run:

- `results/recall_bakeoff_20260516-124537.md`
- `synapse_scoped_fusion_sdk`: R@5 `1.000`, p95 `0.99 ms`
- `cognee_session_recall`: R@5 `1.000`, p95 `7.32 ms`
- `signet_daemon_cli`: R@5 `1.000`, p95 `486.63 ms`

This is a fast local gate. It is not a public LongMemEval/LoCoMo claim.

Note: the full `all` run writes unique benchmark facts into the configured
local memory tools. Use `--engines sqlite,mem0` for a non-persistent smoke run.
