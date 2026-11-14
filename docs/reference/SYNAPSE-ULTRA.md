# Synapse Ultra Memory — v2.0.0

Lean, clean, general-purpose extension for synapse-memory. Adds event log,
graph-v2 (SQLite CTE), observe CLI, and optional DuckLake archive — without
touching the existing synapse-core schema.

## What's new in v2.0.0

- **`synapse-ultra` crate** — additive layer on top of `brain.db`
- **Event log** (`synapse_events`) — beads-style agent action stream with BLAKE3 dedup + zstd
- **Graph-v2** (`graph_nodes` + `graph_edges`) — replaces broken Datalog; recursive CTE traversal
- **`why()` operator** — backward decision-chain (VelesDB-style) via `why_chain` SQL view
- **`graph_expand()`** — forward graph traversal via `graph_expand` SQL view
- **Auto-graph trigger** — inserting a `decisions` row auto-populates graph nodes + edges
- **Token cost log** (`token_cost`) — per-call token usage for cost analytics
- **`synapse-ultra` CLI** — `init / ingest / inspect / why / graph / replay / cost / events / lake / doctor`
- **Ingest scripts** (Python) — `claude-hooks.py`, `codex-usage.py`, `gemini-mcp.py`, `git-post-commit.py`
- **Optional DuckLake** — archive old events to Parquet, time-travel analytics via DuckDB CLI
- **ADRs 0004-0008** — decisions documented

## Quick start

```bash
# 1. Build
cargo build -p synapse-ultra --release

# 2. Initialize Ultra schema on your existing brain.db (idempotent, additive)
./target/release/synapse-ultra init --db ~/.synapse/brain.db

# 3. Ingest events
./target/release/synapse-ultra ingest --db ~/.synapse/brain.db \
  --json '{"agent":"claude","kind":"decision","uri":"file:foo.rs","content":"refactored"}'

# Or from a JSONL file (e.g. produced by scripts/ingest/*.py)
python3 crates/synapse-ultra/scripts/ingest/claude-hooks.py < hook-event.json
./target/release/synapse-ultra ingest --db ~/.synapse/brain.db \
  --jsonl ~/.synapse/ingest/claude.jsonl

# 4. Observe
./target/release/synapse-ultra inspect --db ~/.synapse/brain.db
./target/release/synapse-ultra why --db ~/.synapse/brain.db --uri file:foo.rs --depth 5
./target/release/synapse-ultra graph --db ~/.synapse/brain.db --uri file:foo.rs --dot | dot -Tsvg > graph.svg
./target/release/synapse-ultra replay --db ~/.synapse/brain.db --session sess-abc
./target/release/synapse-ultra cost --db ~/.synapse/brain.db --days 30
./target/release/synapse-ultra events --db ~/.synapse/brain.db --agent claude --kind decision --limit 50

# 5. Optional: DuckLake archive for long-term analytics
brew install duckdb
./target/release/synapse-ultra lake init --db ~/.synapse/brain.db --catalog ~/.synapse/lake/metadata.ducklake
./target/release/synapse-ultra lake archive --db ~/.synapse/brain.db --older-than 90 --catalog ~/.synapse/lake/metadata.ducklake
./target/release/synapse-ultra lake analytics --db ~/.synapse/brain.db --catalog ~/.synapse/lake/metadata.ducklake
```

## Architecture

```
┌────────────────────────────────────────────────────────────┐
│ synapse-ultra (new, ~1500 LOC)                             │
│   events.rs    — Event ingest + BLAKE3 dedup + zstd        │
│   graph.rs     — why() / graph_expand() via SQLite CTE     │
│   observe.rs   — brain_stats / replay / cost_by_day        │
│   lake.rs      — optional DuckLake archive (duckdb CLI)    │
│   schema.rs    — idempotent additive migration             │
└────────────────────────────────────────────────────────────┘
           │ reads/writes (additive)
           ▼
┌────────────────────────────────────────────────────────────┐
│ brain.db (SQLite, WAL)                                     │
│   docs / docs_fts / docs_vec  (synapse-core, unchanged)    │
│   query_logs / meta            (synapse-core, unchanged)   │
│   synapse_events               (new)                       │
│   graph_nodes / graph_edges    (new)                       │
│   decisions / sessions         (new)                       │
│   token_cost                   (new)                       │
│   why_chain / graph_expand     (new SQL views, CTE)        │
└────────────────────────────────────────────────────────────┘
```

## Design principles

1. **Additive only** — never modify existing synapse-core schema
2. **Idempotent migration** — safe to run repeatedly on any brain.db
3. **Single-file storage** — everything in one SQLite file (DuckLake is opt-in)
4. **No new heavy deps** — `rusqlite`, `blake3`, `zstd`, `clap`, `chrono` only
5. **CLI first** — observe via `synapse-ultra` CLI; Astro 7 dashboard later (ADR 0008)
6. **External ingest** — Python hooks write JSONL; CLI loads it. No daemon restarts.

## ADRs

- [ADR 0004 — Synapse Ultra lean extension](../adr/0004-synapse-ultra-lean-extension.md)
- [ADR 0005 — Graph-v2 as SQLite recursive CTE](../adr/0005-graph-v2-sqlite-cte.md)
- [ADR 0006 — Event log as beads-style SQLite table](../adr/0006-events-as-beads-style-sqlite.md)
- [ADR 0007 — DuckLake as optional archive](../adr/0007-ducklake-optional-archive.md)
- [ADR 0008 — Observe CLI now, Astro 7 later](../adr/0008-observe-cli-astro-later.md)

## Testing

```bash
cargo test -p synapse-ultra
```

Key tests:
- `migrate_is_idempotent` — double migration is safe
- `why_chain_traverses_backwards` — 3-node chain
- `why_chain_10k_nodes_under_50ms` — performance regression guard
- `decision_creates_graph_nodes_and_edges` — trigger works
- `ingest_jsonl_file_skips_blank_lines` — robust parser

## Roadmap (v2.1+)

- Astro 7 dashboard (reads the same SQLite views via read-only ATTACH)
- `--json` output for all observe commands
- Live token routing bridge to `token-cfo`
- MCP tools `why` + `graph_expand` exposed via `synapse-mcp`
- WASM target (via `rusqlite` + `sqlite3` compiled to WASM)
