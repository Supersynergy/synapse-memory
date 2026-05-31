# Synapse — Ultimate Agent-Memory Gap Analysis (2026-04-26)

> caveman-body, normal tables. sources inline. no filler.

## TL;DR

- Synapse hat **infrastruktur-moat** (Rust single-file, Ed25519, CRDT, lib-mode 0.69ms, MCP-native) → niemand sonst hat das alle gleichzeitig.
- Synapse fehlt **agent-cognition layer**: hierarchical memory blocks (Letta), temporal bi-temporal graph (Zep/Graphiti), entity-linking + LLM-extraction (mem0 v3), reflection/self-edit, sleep-time compute.
- mem0 v3 (April 2026) hat LoCoMo 91.6 / LongMemEval 93.4 — Synapse hat **keine vergleichbare quality-bench**, nur latency-bench. Quality-bench ist P0.
- Hindsight-parity (banks/IDE-adapters) ist 80% in ADDONS.md geplant, aber Tier 1 noch nicht ausgeliefert. Das ist der schnellste path to adoption.
- Anti-features: KEIN neo4j, KEIN python-runtime-deps, KEIN cloud-only path, KEIN qwen embedder, KEIN GPL-cargo. Bleib bei rust+sqlite+fastembed.

## Synapse current state (verified)

- repo: `~/projects/synapse/`, brain: `~/.synapse/brain.db` (147k docs, 547MB)
- stack: Rust + SQLite + FTS5 + sqlite-vec + fastembed ONNX (BGE-small-384, MLX 4.6× batch)
- bench (lib-mode, M4 Max): 0.69ms hybrid query, 17k put/s, 0.023ms/q v1.0 ranked 3rd after FAISS+FTS5
- features shipped: hybrid FTS5+vec, RRF, MCP server, CLI (`synx`), Ed25519 brainpacks, yrs CRDT, library-mode crate, Thompson+heat self-learning ranker
- planned in PIONEER.md (13 features) + ADDONS.md (6 tiers, hindsight-parity)
- security playbook done: split core(FSL)+engine(closed dylib), per-customer watermark, SQLCipher
- recent milestone 2026-04-25: WP install Success + 4.5ms Lex + 7153 OPS OLTP + MLX 4.6×/doc batch

## Feature Matrix — Synapse vs 6 competitors × 20 features

Legend: ✅ shipped · 🟡 partial/planned · ❌ missing · n/a not-applicable

| Feature | Synapse | Letta | mem0 v3 | Zep | Graphiti | Hindsight | LightRAG | cognee |
|---|---|---|---|---|---|---|---|---|
| Single-file binary, no runtime | ✅ | ❌ py | ❌ py | ❌ srv | ❌ neo4j | ❌ py+ts | ❌ py+docker | ❌ py |
| Hybrid FTS+vec | ✅ | 🟡 | ✅ tri-signal | ✅ | ✅ | ✅ | ✅ | ✅ |
| Knowledge-graph / entity-linking | ❌ | 🟡 | ✅ entity-link | ✅ | ✅ bi-temp | ❌ | ✅ | ✅ ontology |
| Temporal / bi-temporal facts | ❌ | ❌ | ❌ | ✅ | ✅ valid_at/invalid_at | ❌ | ❌ | 🟡 |
| Hierarchical memory blocks (core/archival/recall) | ❌ | ✅ | ❌ | ❌ | ❌ | 🟡 banks | ❌ | 🟡 session+graph |
| Sleep-time / background consolidation | 🟡 P3 plan | ✅ | 🟡 | ✅ | ✅ | ❌ | ✅ doc-del+regen | ✅ improve() |
| Reflection / self-edit | ❌ | ✅ subagents | ❌ | 🟡 | ❌ | ✅ reflect() | ❌ | ✅ |
| Forgetting curve / decay | 🟡 P3 | ❌ | ❌ overwrite-free | 🟡 invalidate | ✅ invalidate | ❌ | ✅ delete | ✅ forget() |
| Multi-tenant / banks / scope | 🟡 ADDONS Tier2 | ✅ users | ✅ user_id | ✅ session | ✅ groups | ✅ banks | 🟡 | ✅ tenant |
| MCP-native | ✅ | 🟡 | 🟡 | ❌ | ❌ | ✅ streamable | 🟡 | 🟡 |
| Multi-writer / CRDT offline | ✅ yrs | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Ed25519 signed memory | ✅ brainpack | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| On-device / local-first | ✅ | 🟡 | 🟡 | ❌ cloud | 🟡 | 🟡 | ✅ | ✅ |
| Latency p95 hybrid | ✅ 0.69ms | ~50ms | 0.88s | sub-s | sub-s | ~10ms | ~100ms | sub-s |
| Quality bench (LoCoMo/LongMemEval) | ❌ none | 🟡 | ✅ 91.6/93.4 | ✅ SOTA | ✅ | ❌ | 🟡 | ❌ |
| LLM extraction pipeline | ❌ | ✅ | ✅ single-pass ADD | ✅ | ✅ | 🟡 reflect | ✅ | ✅ cognify |
| Cross-encoder / rerank | 🟡 P1 plan | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ default | ❌ |
| IDE/agent adapters (claude-code/cursor/aider) | 🟡 ADDONS Tier1 | ✅ letta-code | 🟡 cli | ❌ | ❌ | ✅ 6 adapters | 🟡 webui | 🟡 |
| SDKs (py/ts/rust/go) | 🟡 rust+lib | ✅ py/ts | ✅ py/ts | ✅ py/ts | ✅ py | ✅ ts/py | ✅ py | ✅ py |
| Live timeline / pub-sub | 🟡 P2 plan | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |

Source: `~/.claude/projects/-Users-master/memory/reference_memory_systems_2026.md`, mem0 README (April 2026), Graphiti README, Letta README, Hindsight ghgrep, LightRAG README, cognee README.

## Was Synapse BRAUCHT (priorisiert)

### P0 — ship in 2 weeks oder game over

1. **LLM-Extraction-Pipeline (mem0-style ADD-only)**
   why: ohne entity-extraction kann Synapse keine "memory" für agents — nur "search". mem0 v3 LoCoMo 91.6 kommt von single-pass extract+entity-link. Ohne das verlierst du adoption-war.
   impl: `synx extract --llm <local-mlx>` → entities + facts + relations → store as typed `[memory]` events with entity-refs. Reuse fastembed for entity-embed.
   bench-target: LoCoMo ≥ 85, LongMemEval ≥ 88 (within 5pt of mem0 v3).

2. **Quality-Benchmark Suite (LoCoMo + LongMemEval + BEAM)**
   why: Synapse hat nur latency-bench. mem0 v3 README zeigt quality-bench mit zahlen. Ohne quality-zahlen wirst du für "embedded vector store" gehalten, nicht für "agent memory". P0 weil ALLE marketing-claims darauf basieren.
   impl: `eval/locomo_runner.rs`, fork mem0/memory-benchmarks (MIT), produce `bench/quality_2026_04.md`.

3. **Tiered Hybrid Rerank (PIONEER P1)**
   why: ohne cross-encoder-rerank bleibst du 10-15 recall@10 punkte hinter Zep+mem0. Bereits in PIONEER.md. impl-skizze ready.
   impl: L1 BM25 → L2 vec → L3 phi-4-mini MLX cross-encoder (only if margin<ε) → L4 bandit fusion.

### P1 — ship in 4-6 weeks

4. **Bi-temporal facts (`valid_at`/`invalid_at`)**
   why: Graphiti's killer-feature laut paper (arxiv 2501.13956) — agents brauchen "wann war das wahr". Lex-feature für enterprise (compliance/audit).
   impl: 2 SQLite cols pro fact-row, FTS5-filter `WHERE valid_at <= :t AND (invalid_at IS NULL OR invalid_at > :t)`. Auto-invalidate on contradiction (LLM-judge).

5. **Hindsight-parity Tier 1 (synapse-openclaw + claude-code + cursor)**
   why: ADDONS.md Tier 1, blockiert adoption. Hindsight hat 6 adapters → 6× distribution.
   impl: TS wrappers über MCP, `retain/recall/reflect` → `put/search/timeline` mapping.

6. **Hierarchical memory-blocks (Letta-style core/archival/recall)**
   why: Letta-architektur ist standard für agent-frameworks. core=working set in prompt, archival=full searchable, recall=conversation. Ohne das kein letta/agno/swarm-integration.
   impl: 3 tabellen-views auf brain.db, MCP tools `core_memory_*`, `archival_*`, `recall_*` API-kompat zu Letta.

7. **Sleep-time consolidation daemon**
   why: PIONEER P3 + competitor-feature. Decay+compress low-score memories via local LLM (phi-4-mini MLX). cognee/Letta/Zep haben das.
   impl: `synx daemon` launchd job, jede nacht 03:00, frequency*recency*diversity → score → batch-summarize bottom 10%.

### P2 — moat-deepening, ship Q3 2026

8. **Multi-agent live timeline (PIONEER P2)** — pub/sub websocket, kein konkurrent.
9. **Semantic CRDT merge (PIONEER P2)** — chunk-boundary merge mit local LLM dispute-resolver.
10. **Federated bandit DP (PIONEER P3)** — privacy-preserving cross-user ranking learn.
11. **WASM/browser target (PIONEER P3)** — embed in chrome-ext, no competitor has.
12. **OpenTelemetry / audit trail** — cognee hat OTEL collector, enterprise-must-have.

## Was Synapse NICHT braucht (anti-features)

- ❌ **Neo4j-anything** — Graphiti/Zep blocker. SQLite-graph (recursive CTE + JSON) reicht für 99% workloads.
- ❌ **Python-runtime-deps** — bricht single-file moat. Letta/mem0/cognee leiden darunter.
- ❌ **Cloud-only-path** — Zep cloud-trap. Local-first ist USP.
- ❌ **Qwen-embedder** — memory-rule: "ALL Qwen verboten". BGE-small ONNX bleibt.
- ❌ **AutoGPT/AutoGen tight-coupling** — bleib protocol-layer (MCP), nicht framework-fork.
- ❌ **JS-runtime im core** — TS nur in adapters (Tier 1).
- ❌ **GPL/AGPL deps** — playbook sagt FSL/Apache. crateaudit before merge.
- ❌ **"Memory as a chatbot UI"** — kein webui im core. cognee/Hindsight/LightRAG verschwenden energie da.
- ❌ **Eigener Vektor-index (ANN-from-scratch)** — sqlite-vec ist proven. Nicht in faiss-territory einsteigen außer als P3 IVF-PQ-overlay.
- ❌ **Multi-DB-backend (postgres/mongo)** — bricht single-file. mem0 leidet.

## Top 10 stealable patterns (200 repos scan)

1. **mem0 single-pass ADD-only extraction** — github.com/mem0ai/mem0 — eine LLM-call statt agentic loop, +20pt LoCoMo. Steal: prompt + entity-linking-merge.
2. **Graphiti bi-temporal facts** — github.com/getzep/graphiti — `valid_at`/`invalid_at` + auto-invalidation. Steal: schema + contradiction-detector.
3. **Letta core/archival/recall blocks** — github.com/letta-ai/letta — 3-tier memory API. Steal: API shape, port to MCP.
4. **Letta-code skills+subagents** — github.com/letta-ai/letta-code — agent-side memory CLI. Steal: `letta` ergonomics for `synx agent`.
5. **Hindsight bank-scoping + MCP streamable** — github.com/vectorize-io/hindsight — header-based multi-tenancy. Steal: `X-Bank-Id` pattern.
6. **cognee 4-verb API (remember/recall/forget/improve)** — github.com/topoteretes/cognee — minimaler, sehr lesbar. Steal: CLI verb-naming.
7. **LightRAG reranker as default mode** — github.com/HKUDS/LightRAG — set-default cross-encoder, big quality jump. Steal: default-on for queries with margin<ε.
8. **mem0 memory-benchmarks repo** — github.com/mem0ai/memory-benchmarks (MIT). Steal: fork as `synapse-bench-quality`.
9. **Zep arxiv 2501.13956** — temporal-KG architecture paper. Steal: schema + ablations to reproduce.
10. **claude-subconscious sync_letta_memory.ts** — github.com/letta-ai/claude-subconscious — claude-code↔letta sync pattern. Steal: directly for `synapse-claude-code`.

## 90-Day Roadmap

**Days 0-14 (P0 sprint)**:
- D1-3: fork mem0/memory-benchmarks → `eval/quality/`. Wire LoCoMo runner. Baseline-bench Synapse current.
- D4-9: implement extraction pipeline (`synx extract` + entity-link). Re-bench.
- D10-14: tiered rerank L3 cross-encoder (phi-4-mini MLX). Re-bench. Target ≥85 LoCoMo.
- Deliverable: `bench/quality_2026_05_v1.md` + blogpost vs mem0 v3.

**Days 15-45 (P1 sprint)**:
- D15-21: bi-temporal schema + contradiction-LLM-judge. Migration script for brain.db.
- D22-30: Tier-1 adapters: synapse-openclaw, synapse-claude-code, synapse-cursor (TS wrappers).
- D31-38: hierarchical memory-blocks (core/archival/recall) MCP API + Letta-compat shim.
- D39-45: sleep-time consolidation daemon + decay scoring. launchd integration.
- Deliverable: Show HN — "Synapse 2.0: Mem0-quality + Letta-API + 0.69ms latency, in one Rust binary."

**Days 46-90 (moat sprint)**:
- D46-60: live timeline pub/sub + multi-agent demo (2 agents shared memory).
- D61-75: semantic CRDT merge + sync between brainpacks. WASM target prototype.
- D76-90: OTEL collector, audit trail, federated bandit DP. Secure release per playbook.
- Deliverable: 1.0 commercial release per SECURE_RELEASE_PLAYBOOK; HN+r/LocalLLaMA+r/rust launch.

## Risiken

- **License-creep**: Letta is Apache, mem0 is Apache, Graphiti is Apache, LightRAG MIT, cognee Apache → safe to study, NOT to copy code 1:1. Re-implement APIs from spec, never paste source. crateaudit before merge.
- **Complexity-creep**: bi-temporal + KG + reflection könnte Synapse's "single-file moat" gefährden. Halte alles als optional features hinter cargo-flags (`--features kg,temporal,reflect`).
- **Perf-regression**: cross-encoder rerank +30ms, extraction +1-2s pro put. Lösung: rerank only on margin<ε, extraction async via timeline pub/sub (return synchronous ack, extract in bg).
- **Bench-trap**: Wenn LoCoMo <80 nach P0-sprint → DON'T LAUNCH. mem0 v3 quality-claim ist scharf. Ehrlicher Vergleich ist USP, gefälscht = death.
- **Adoption-trap**: Hindsight hat 6 adapters in production. Wenn Synapse-Tier-1 nicht 6 weeks vor 1.0 fertig ist, verlieren wir das adopter-fenster.
- **Embedder-cutoff**: BGE-small-384 könnte 2026 obsolet werden. Halte fastembed-backend abstract, watch HF-trends-feeds.

## Quellen

- `~/projects/synapse/PIONEER.md` (verified)
- `~/projects/synapse/ADDONS.md` (verified)
- `~/projects/synapse/SECURE_RELEASE_PLAYBOOK.md` (verified)
- `~/.claude/projects/-Users-master/memory/reference_memory_systems_2026.md`
- mem0 README (github.com/mem0ai/mem0, fetched 2026-04-26)
- Graphiti README + arxiv 2501.13956
- Letta README + letta-code repo
- Hindsight (vectorize-io) via ghgrep
- LightRAG README (HKUDS)
- cognee README (topoteretes)
