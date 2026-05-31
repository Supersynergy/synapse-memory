# Recall Bakeoff: Synapse vs Local Candidates

Run: `RBK-PUBLIC-20260516-134220`
Golden records: `8`; queries: `16`; top-k: `5`
Raw JSON: `recall_bakeoff_20260516-134224.json`

## Scoreboard

| Engine | Status | R@1 | R@5 | MRR | p50 ms | p95 ms | Ingest ms | Notes |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| synapse_scoped_fusion_sdk | ok | 1.0 | 1.0 | 1.0 | 0.34 | 0.48 | 0.0 | indexed scope-first recall, BatchSearch fallback, docs.meta filter, query-term rerank |
| synapse_mem0_compat_socket | ok | 1.0 | 1.0 | 1.0 | 0.64 | 1.3 | 816.99 | Synapse-backed drop-in mem0 API, local socket, Lex mode, no cloud |
| synapse_hybrid_socket | ok | 1.0 | 1.0 | 1.0 | 13.84 | 18.86 | 872.53 | global Synapse daemon via Unix socket |
| sqlite_fts5_control | ok | 0.125 | 0.625 | 0.285 | 0.02 | 0.04 | 0.27 | local lexical control |
| claude_mem_worker_import_fts | ok | 0.0 | 0.0 | 0.0 | 0.54 | 1.97 | 8.62 | claude-mem 13.2.0 worker, isolated temp profile, Chroma disabled, /api/import + /api/search?format=json; import stats: {'sessionsImported': 1, 'sessionsSkipped': 0, 'summariesImported': 0, 'summariesSkipped': 0, 'observationsImported': 8, 'observationsSkipped': 0, 'promptsImported': 0, 'promptsSkipped': 0} |

## What The Run Actually Tests

- Real write path plus real query path for the selected engines: sqlite_fts5_control, synapse_hybrid_socket, synapse_scoped_fusion_sdk, synapse_mem0_compat_socket, claude_mem_worker_import_fts.
- Unique per-run expected tokens avoid false wins from the existing large Synapse corpus.
- This is a small precision/latency gate, not a public LongMemEval/LoCoMo claim.

## Architecture Patterns That Win

1. Hot path and deep path must be separate: Synapse should keep the socket/FTS/vector path tiny, while graph/agent tools run behind it as enrichment.
2. Local-first docs freshness should be a version-resolved evidence layer: local package docs first, remote MCP docs second, training memory last.
3. Recall needs lifecycle semantics: temporal validity, supersedes/conflicts edges, and stale duplicate dampening are more valuable than just more vectors.
4. Query routing should be learned from outcomes: accepted context, opened files, edits, test pass/fail, and user correction create the best reward signal.
5. Batch/keepalive beats micro-optimizing per-call work once fork/connect dominates; this is the cleanest near-term Synapse latency lever.
6. Token savings are a retrieval quality problem: pack fewer, fresher, higher-confidence facts by task mode instead of dumping more context.

## Best Next Architecture

Synapse should be the primary local recall kernel. OMEGA contributes memory lifecycle ideas, Signet contributes portable identity/structured memory ideas, Letta contributes core-vs-archival agent memory, Cognee/Graphiti contribute graph/temporal enrichment, and Context7/Docfork/GitMCP/DeepWiki contribute freshness evidence. The winning shape is not replacing Synapse; it is a router that keeps Synapse hot and calls the others only when they add measurable value.
