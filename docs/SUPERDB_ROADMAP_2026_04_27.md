# Synapse → Universal Agent Superdatabase

**Status: 2026-04-27** · Author: Maxim Supersynergy

## Was Synapse JETZT kann (verified)

### Layer 1 — Synapse-ultra (`/tmp/synapse-ultra.sock`, msgpack)
- 147k docs, 547MB, hybrid FTS5+vec, p95 4ms
- Ed25519-signed releases, single-file Rust, no Neo4j
- Drop-in: WP install Success, 4.5ms Lex, 7153 OPS OLTP
- MLX 4.6× embed batch
- Bench 0.023ms/query (3rd nach FAISS+FTS5)

### Layer 2 — Cognition (`~/.synapse/cognition.db`, intent-manager)
- Bi-temporal triple-store (Graphiti pattern, no Neo4j)
- Tiers: episodic / semantic / procedural / core
- FTS5 lexical + decay-weighted (Zep-style forgetting curve)
- Adapter-Trait: Letta / mem0 / Hindsight slots
- 4-tier intent cascade: keyword(5µs) → example-FTS5(1ms) → synapse-ultra(8ms) → MLX(0.7s)
- LLM-extraction via MLX daemon (Phi-4-mini-instruct-4bit, warmup'd)
- L3 cross-encoder rerank, margin-gated
- perf_event log → CatBoost-ready training fuel
- launchd: nightly decay (03:15), MLX daemon (always-on)
- Hooks: UserPromptSubmit (intent-suggest) + PostToolUse (intent-reward)

### Verified speed
| Op | Latency | Notes |
|----|---------|------|
| FTS5 keyword | <1ms | bm25 + decay |
| Synapse-ultra hybrid | 4ms | 147k corpus |
| Intent classify T0 | ~5µs | AhoCorasick cached |
| Intent classify T2 (ultra) | 67ms | with msgpack roundtrip |
| MLX extract (warm) | 0.65s | early-stop on `]` |
| MLX rerank L3 | ~0.7s | batched single call |

## Gap vs "Universal Agent Superdatabase"

| Gap | Impact | Effort |
|-----|-------|-------|
| **No multi-agent IPC layer** — only Claude Code wired via hooks | 🔴 P0 | 1d |
| **No MCP server** — agents speaking MCP can't reach it | 🔴 P0 | 1d |
| **No HTTP/REST surface** — non-MCP agents (ZeroClaw, OpenFang, ruflo) blocked | 🔴 P0 | 0.5d |
| **No agent-isolation semantics** — all agents share `agent` column but no ACL | 🟡 P1 | 2d |
| **No conflict resolution** — concurrent writes race ON CONFLICT | 🟡 P1 | 1d |
| **No CRDT sync** — multi-machine deployments diverge silently | 🟡 P1 | 3d |
| **No quality bench gate** — quality regressions ship unseen | 🔴 P0 | 1d |
| **No semantic dedup at ingest** — only blake3 exact-hash | 🟡 P1 | 1d |
| **No streaming subscribe** — agents can't react to new memories | 🟡 P1 | 1d |
| **No timeline UI** — humans can't audit agent memory | 🟢 P2 | 3d |

## 90-Day Plan

### Week 1 (P0 distribution)
- **D1: HTTP server** — `intent serve --port 9477` exposes classify/recall/remember as JSON-RPC. Bind localhost + Bearer-token auth. 4-route surface, ~150 lines.
- **D2: MCP server** — `intent mcp` stdio adapter exposes 6 tools (classify, recall, remember, forget, list_intents, ingest). MCP SDK Rust binding or thin shim.
- **D3: Bench harness wired** — fork mem0/memory-benchmarks fixtures, run nightly, persist to bench_result, fail-CI on regression.
- **D4-5: Adapter wire-tests** — Letta API surface, mem0 ADD endpoint, Hindsight 3-method contract; one integration test each.

### Week 2-3 (P1 reliability)
- Conflict-resolution: optimistic locking via `tx_time` CAS column on UPDATE
- Per-agent ACL: agent_keys table + Bearer-token-per-agent middleware
- Semantic dedup: cosine threshold 0.92 BEFORE blake3 insert (uses fastembed-384)
- Streaming SUB: SQLite update_hook → unix-socket fanout `/tmp/synapse-events.sock`

### Week 4-6 (P2 scale)
- CRDT sync: Automerge or Loro for multi-machine. Append-only log + periodic compaction.
- Timeline UI: tiny SvelteKit app, reads cognition.db read-only, plots memories per agent on bi-temporal axes
- WASM build: cognition.db queryable from browser/Edge worker
- OTEL traces: every classify/recall emits OTLP spans → Tempo/Jaeger

### Week 7-12 (Differentiation)
- **Agent-of-Agents memory** — share semantic tier across agents, isolate episodic
- **Reflection job** — nightly, MLX summarizes top-N episodic per agent → distilled semantic facts (Letta sleep-time-compute pattern)
- **Goal-conditioned recall** — agent passes current_goal, retrieval reranks by goal-relevance not just query
- **Cost-per-agent tracking** — perf_event extends with token_cost; bandit picks cheapest tier hitting quality

## Multi-Agent Architecture Design

```
                  ┌────────────────────────────┐
                  │   intent-manager binary    │
                  │   ───────────────────       │
                  │   serve --port 9477        │ ←── ALL agents
                  │   mcp (stdio)              │ ←── MCP-speaking agents
                  │   classify/recall/remember │ ←── CLI users
                  └──────────┬─────────────────┘
                             │
                  ┌──────────▼─────────────────┐
                  │  cognition.db (SQLite)     │
                  │  bi-temporal triple-store  │
                  │  per-agent partition column│
                  └──────────┬─────────────────┘
                             │
                  ┌──────────▼─────────────────┐
                  │  synapse-ultra (msgpack)   │
                  │  vector recall, 147k docs  │
                  └────────────────────────────┘

   Agents using it:
     - Claude Code      (hook-based, native)
     - ZeroClaw         (HTTP)
     - OpenFang         (HTTP)
     - ruflo            (HTTP)
     - Cursor/Devin     (MCP)
     - Custom Rust apps (lib import)
```

## Anti-Goals (was es NICHT wird)

- **Kein Neo4j-Konkurrent** — graph queries via SQLite recursive CTE, ausreichend bis 10M edges
- **Kein Vector-DB-Konkurrent** — synapse-ultra macht Vec, cognition.db macht Triples
- **Kein Vendor-Lock** — Single SQLite file, exportable in 1 line
- **Kein Cloud-only** — alles localhost-first, Sync optional
- **Kein Python-Hot-Path** — MLX daemon ist ausgelagert, core bleibt Rust
- **Kein LangChain-Bloat** — Adapter sind 30-Zeilen Trait-Impls

## Konkurrenz-Positionierung

| Feature | Synapse-Cognition | mem0 | Letta | Zep | Graphiti | Hindsight |
|---------|-------------------|------|-------|-----|----------|-----------|
| Bi-temporal | ✅ | ❌ | ❌ | partial | ✅ | ❌ |
| Single-file | ✅ | ❌ Postgres+Qdrant | ❌ Postgres | ❌ Postgres | ❌ Neo4j | ✅ |
| Forgetting curve | ✅ | ❌ | ❌ | ✅ | ❌ | ❌ |
| Agent isolation | partial | ✅ | ✅ | ✅ | ✅ | ❌ |
| MCP server | ⏳ Week 1 | ❌ | ❌ | ❌ | ❌ | ❌ |
| HTTP REST | ⏳ Week 1 | ✅ | ✅ | ✅ | ✅ | ❌ |
| MLX-native | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Cost-aware cascade | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Sub-1ms recall | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Rust core | ✅ | Python | Python | Python | Python | Python |

## Konkretes "ship next"

D1 (heute): HTTP server. Andere Agenten reden mit Synapse via:
```bash
curl -H "Authorization: Bearer $T" -d '{"prompt":"fix auth"}' \
     http://localhost:9477/classify
```

Below ist Phase 1 done. Andere Agenten read+write ohne Code-Changes.
