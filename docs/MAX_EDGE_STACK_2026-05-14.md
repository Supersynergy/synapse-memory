# Max Edge Stack Blueprint - 2026-05-14

## Basis

Inputs:
- Synapse precheck: AwesomeIndexer/Rust edge stack memories and broken list.
- Broken/avoid: chromadb, weaviate, LangChain, Selenium.
- gh-grep/ghmax mining:
  - `cocoindex.FlowBuilder` in CocoIndex examples and code indexing flows.
  - `impl ProxyHttp for` in Cloudflare Pingora examples.
  - `tantivy::Index::create_in_ram` in multiple Rust search projects.
  - `sonic_rs::from_slice` in API/MCP/server hot paths.
  - `Qdrant::from_url` and `VectorParamsBuilder::new` in Rust vector-store integrations.
  - `LazyFrame` in Polars/Nushell SQL/lazy integrations.
- superscrape:
  - CocoIndex codebase RAG docs: incremental processing, Tree-sitter chunking, stable flow/export declarations.
  - CocoIndex flow docs: `refresh_interval`, `max_inflight_rows`, `max_inflight_bytes`, stable target names, generated UUIDs.
  - Pingora docs.rs: `ProxyHttp` callbacks for request filtering, upstream routing, cache filters, body filters, logging.

## Two Highest Levers

1. **Compounding source delta loop**
   Treat every input as a source with stable identity, lineage, and incremental update state. This is the CocoIndex pattern, but Synapse should own the local memory/output plane.

2. **Outcome-instrumented local brain**
   Every fetch, recall, score, alert, accept/reject, and later user correction becomes a compact outcome event. This gives SuperML/Autolearn real reward data instead of vibes.

## Target Architecture

```text
authorized sources
  -> edge collector (wreq/webclaw now, Pingora only where cache/rate-limit pays)
  -> normalizer (readability/html/pdf/code/tree-sitter)
  -> delta ledger (stable source_id + content_hash + lineage)
  -> Synapse hot memory (FTS5/Tantivy + vec + entity/temporal graph)
  -> optional Qdrant/Lance tier only above local scale threshold
  -> Polars/DuckDB score/report plane
  -> alert/report/CLI
  -> outcome log
  -> autolearn bandit updates routing/thresholds/budgets
```

## Pattern Transfers

| Pattern | Source signal | Synapse transfer |
|---|---|---|
| Stable flow declarations | CocoIndex `@flow_def`, `FlowBuilder`, `DataScope` | Add a local `source_flow` ledger: stable source name, filters, transform version, target name. |
| Incremental update only | CocoIndex docs: changed rows only | Never recrawl/reembed unchanged content; make content hash and transform hash first-class. |
| Concurrency budgets | CocoIndex `max_inflight_rows/bytes` | Put hard budgets on source rows, bytes, embeddings, and per-host fetches. |
| Semantic code chunks | CocoIndex Tree-sitter example | Prefer syntax-aware code chunks for agent memory over arbitrary line windows. |
| Proxy callbacks | Pingora `ProxyHttp` | Use Pingora for collector gateway/cache/rate-limit, not for simple Synapse hot API. Existing local bench says simple Pingora hop costs throughput. |
| In-process search | Tantivy `create_in_ram`, reader reload, writer budget | Use embedded Tantivy for lex-heavy shards and warm readers; keep FTS5 where it already wins on simplicity. |
| Fast JSON parse | `sonic_rs::from_slice` | Use typed fast parse for high-volume JSONL/API event ingest; keep `serde_json` for low-volume CLI/admin paths. |
| Vector client builder | Qdrant `from_url`, timeout, compression, vector params | Treat Qdrant as optional scale-out semantic tier, not the default local brain. |
| Lazy analytics | Polars `LazyFrame` logical plan | Score batches lazily, materialize only final reports/features. |

## Stack Verdict

| Rank | Component | Why |
|---:|---|---|
| 1 | Synapse batch recall + context portfolio | Already installed; turns one prompt into multi-perspective recall without N process forks. |
| 2 | CocoIndex-style source_flow ledger | Biggest missing compounding layer: lineage, delta, stable target state. |
| 3 | Outcome/event ledger | Required for autolearn rewards, threshold tuning, and fair benchmark truth. |
| 4 | Tantivy lexical tier | Best embedded lex/search upgrade when FTS5 hits quality limits. |
| 5 | Polars/DuckDB score plane | Turns raw evidence into ranked alerts and business reports. |
| 6 | sonic-rs on JSON hot paths | Cheap speed win for event-heavy collectors. |
| 7 | Pingora edge collector | Use only when cache/rate-limit/proxy lifecycle is worth added complexity. |
| 8 | Qdrant/Lance optional external tier | Good escape hatch above single-node/local-memory scale. |

## SuperML/Autolearn Application

Do not train a model first. Create rewardable arms:

| Routing arm | Reward |
|---|---|
| `fetch_strategy`: wreq, curl_cffi, browser, cached | success_rate - latency_penalty - block_penalty |
| `chunk_strategy`: line, paragraph, tree-sitter, semantic | recall_hit + compression_saved - hallucination_penalty |
| `recall_strategy`: primary, multi-perspective, HyDE, graph-hop | accepted_context + task_success - token_cost |
| `alert_threshold`: strict, balanced, broad | user_accept + downstream_action - false_positive |
| `storage_tier`: Synapse-only, Synapse+Qdrant, Synapse+Lance | recall@k + p95_budget - ops_cost |

Minimum viable reward event:

```json
{
  "ts": 0,
  "flow": "source_flow_name",
  "arm": "multi_perspective_recall",
  "query_hash": "blake3",
  "latency_ms": 0.0,
  "tokens": 0,
  "accepted": true,
  "task_success": null,
  "notes": "optional compact text"
}
```

## Ship Order

1. **This week: source_flow ledger**
   Add a small SQLite table or Synapse doc convention for source URL/path, include/exclude filters, transform version, content hash, last_seen, last_changed.

2. **This week: outcome ledger**
   Log every context injection and alert with arm, latency, token budget, and user/action outcome when available.

3. **Next: tree-sitter chunker for code memory**
   Mine CocoIndex chunking shape, but keep local ownership: syntax-aware chunks written to Synapse with file path, symbol, byte ranges.

4. **Next: score-plane CLI**
   `synapse signal run <flow>` should output ranked Markdown/JSON and write outcome-ready events.

5. **Later: Pingora collector**
   Only after benchmarks show collector-side cache/rate-limit wins. Keep `synapsed` itself on the faster direct path.

## Do Not Do

- Do not replace Synapse with Qdrant. Use Qdrant as optional remote semantic tier.
- Do not add LangChain. Direct pipelines are clearer and faster.
- Do not put Pingora in front of every local call. The local benchmark already says this is not free.
- Do not let crawlers write raw noise to memory. Source deltas first, normalized evidence second, compact decisions third.
- Do not claim SOTA from synthetic routing simulations. Use BEAM/LongMemEval/LoCoMo only after a fair harness run.

## Verification Helper

Run:

```bash
python3 tools/edge_stack_score.py
python3 tools/edge_stack_score.py --json
python3 integrations/claude-code/hooks/synapse_context.py --learn-stats
printf '{"prompt":"latest serde API version","cwd":"%s"}' "$PWD" | python3 integrations/claude-code/hooks/synapse_context.py --fresh-context
synx fresh-context --cwd "$PWD" --prompt "latest serde API version" --max-registry 1
python3 tools/source_flow_ledger.py stats
```

The score is not a benchmark. It is a leverage prior for sprint ordering. Replace priors with measured reward as soon as enough outcome data exists.

## Implemented Slice

- `synx batch` keepalive recall is installed as the low-latency multi-query path.
- `synapse-core::fresh` is now the native Freshness Guard: manifest/lockfile scan, version-pinned docs URLs, broken/avoid hints, SQLite registry cache, and optional crates.io/npm/PyPI latest checks.
- `synx fresh-context` exposes that guard directly for hooks, agents, shell use, and future MCP/daemon paths. It accepts `--prompt`, `--cwd`, `--project`, `--mode`, `--json`, `--no-registry`, and `--max-registry`.
- `synapse_context.py` now logs compact recall events and adapts perspective weights from Stop-hook outcomes.
- `synapse_context.py` now injects a Freshness Guard for package/API work and prefers the native `synx fresh-context` implementation when available, with Python fallback for older binaries.
- `stop_extract.sh` now feeds success/failure signals back into the local recall learner without storing full transcripts.
- `tools/source_flow_ledger.py` implements the local source delta ledger: stable source config, include/exclude filters, transform version, content hashes, scan runs, changed/removed previews.

Freshness defaults:
- Prompt mode checks up to 5 registry packages with a `750 ms` HTTP timeout.
- Session mode checks up to 3 registry packages with a `250 ms` HTTP timeout.
- Registry hits cache for `21600 s`; unknown/failed latest checks cache for `60 s`.
- `SYNAPSE_FRESH_NATIVE=0` forces hook fallback; `SYNAPSE_FRESH_NO_REGISTRY=1` keeps local lockfile-only context.

Freshness Guard microbench on this repo after cache warmup:
- Registry disabled: p50 `2.890 ms`, p95 `3.282 ms`, p99 `3.524 ms` over 100 runs.
- Cached registry enabled: p50 `3.375 ms`, p95 `3.900 ms`, p99 `4.090 ms` over 100 runs.
