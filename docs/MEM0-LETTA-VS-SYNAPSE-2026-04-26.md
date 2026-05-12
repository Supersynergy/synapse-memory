# Synapse vs mem0 vs Letta — Brutal Capability Audit

**Date**: 2026-04-26
**Sources**: github.com/mem0ai/mem0 (README, Apr 2026 v3 algorithm), github.com/letta-ai/letta (README), Synapse internal docs (`NICHE-DOMINANCE-2026-04-26.md`, `KRASS-REBASE-PLAN-2026-04-26.md`), Synapse measured benches.

---

## Section 1 — Capability Matrix

| Capability | mem0 v3 (Apr 2026) | Letta (formerly MemGPT) | Synapse |
|---|---|---|---|
| **Add memory** | ✅ ADD-only single-pass | ✅ via tools (core/archival) | ✅ |
| **Search memory** | ✅ semantic+BM25+entity fused | ✅ archival_memory_search | ✅ vec+FTS5+RRF |
| **Get/Update/Delete** | ✅ (UPDATE/DELETE deprecated in v3) | ✅ memory block edit tools | ✅ |
| **History/audit log** | ✅ | ✅ message+state log | partial (CRDT log, no UI) |
| **Reflect/consolidate** | ❌ (removed in v3 — accumulate-only) | ✅ LLM-driven self-edit | ❌ |
| **Rerank** | ✅ multi-signal fusion | partial | ✅ RRF |
| **Episodic memory** | ✅ messages | ✅ recall_memory (msg history) | ❌ |
| **Semantic memory** | ✅ extracted facts | ✅ archival_memory | ✅ |
| **Procedural memory** | ❌ | ✅ persona/system block | ❌ |
| **Working/scratchpad** | ❌ | ✅ core memory blocks (in-context) | ❌ |
| **Hierarchical memory (MemGPT pattern)** | ❌ | ✅ flagship feature | ❌ |
| **Agent-state persistence** | partial | ✅ first-class | ❌ |
| **Vec store** | ✅ (Qdrant/PGvector/many) | ✅ pgvector / Chroma | ✅ sqlite-vec embedded |
| **Graph store** | ✅ (Neo4j/Memgraph optional) | ❌ | ❌ |
| **KV/SQL** | ✅ via providers | ✅ Postgres | ✅ SQLite |
| **Single-binary embedded** | ❌ (server+vec+graph) | ❌ (server+postgres) | ✅ |
| **LLM required at write** | ✅ (extracts facts) | ✅ (agent loop) | ❌ (raw store) |
| **Configurable LLMs** | ✅ many via litellm | ✅ model-agnostic | n/a |
| **Default embedder** | OpenAI text-embedding-3-small | OpenAI / configurable | local 384-dim ONNX |
| **Local-only mode** | partial (needs LLM) | partial | ✅ fully offline |
| **Server required** | optional (lib OR docker) | ✅ required (Letta server) | ❌ optional |
| **Cloud SaaS** | ✅ app.mem0.ai | ✅ app.letta.com | ❌ |
| **Multi-tenancy** | ✅ user_id/agent_id/run_id | ✅ agent_id (user via app) | ✅ session-isolation |
| **Auth/API-key** | ✅ (server, on-by-default) | ✅ | partial (Ed25519 sign) |
| **Encryption-at-rest** | ❌ (provider-dep) | ❌ | partial (signed, not encrypted) |
| **Signed memory provenance** | ❌ | ❌ | ✅ Ed25519 |
| **CRDT replication** | ❌ | ❌ | ✅ |
| **MCP-native** | ❌ (community) | ❌ | ✅ |
| **MySQL wire-protocol** | ❌ | ❌ | ✅ 1078 QPS |
| **Sub-ms p50 search** | ❌ (0.88s LoCoMo incl LLM) | ❌ (server+pgvector) | ✅ 0.7ms cached |
| **Recall@10 @ 100k** | ~0.92 LoCoMo | not published | ✅ 1.000 exact KNN |
| **Pure-Rust core** | ❌ Python | ❌ Python | ✅ |
| **Pip install** | ✅ `mem0ai` | ✅ `letta-client` | ❌ |
| **License** | Apache 2.0 | Apache 2.0 | TBD |

---

## Section 2 — What mem0 Has That Synapse Doesn't

1. **LLM-side fact extraction on write** (single-pass ADD). Effort: 2 days — pluggable extractor, calls Ollama/API, stores raw+extracted.
2. **Entity linking across memories** (entity boost in retrieval). Effort: 4 days — NER pass, entity table, join-boost in T1.
3. **Multi-signal scoring fusion (sem+BM25+entity)** as a documented contract, not just RRF. Effort: 2 days — formalize + benchmark on LoCoMo.
4. **Graph backend option** (Neo4j/Memgraph). Effort: 1 week — adapter; OR build embedded property-graph on SQLite.
5. **Public benchmark numbers on LoCoMo/LongMemEval/BEAM**. Effort: 3 days — run benchmarks, publish.

## Section 3 — What Letta Has That Synapse Doesn't

1. **Hierarchical memory (core in-context + archival + recall)** — the MemGPT moat. Effort: 1 week — block manager + auto-eviction + tool-call surface.
2. **Memory blocks as named, LLM-editable scratchpads** (`human`, `persona`, custom). Effort: 3 days — typed block table + edit-tool MCP surface.
3. **Stateful agent abstraction** (agent_id with persistent state, not just memory). Effort: 1 week — agent_state table + lifecycle.
4. **Tool-calling integration** (memory ops as tools the LLM picks). Effort: 2 days — already MCP-native, just expose ops as tools.
5. **Self-editing memory via LLM reflection loop**. Effort: 1 week — scheduled reflect job, LLM rewrites.

## Section 4 — What Synapse Has That Neither Has

1. **Embedded single-binary, zero-server** (mem0 needs LLM+vec+graph services; Letta needs Letta-server+Postgres). Synapse: 1 SQLite file.
2. **Sub-ms p50 (0.7ms cached)** — mem0 publishes 0.88-1.09s p50 (incl LLM); Letta has roundtrip+pg latency. Synapse is **>1000× faster** on raw retrieval.
3. **Ed25519 signed memory + CRDT replication** — neither has cryptographic provenance or multi-writer merge.
4. **MySQL wire-protocol surface (1078 QPS @ 8 threads)** — query memory from any SQL client.
5. **Pure-Rust + simsimd_rayon SIMD** — 5.4× over BLAS, no Python GIL, deployable to edge/embedded.

## Section 5 — Performance Reality

| System | p50 latency | What's measured | Server hop? |
|---|---|---|---|
| mem0 v3 | 880ms (LoCoMo) | retrieval + LLM extract | ✅ |
| mem0 v3 | 1090ms (LongMemEval) | retrieval + LLM extract | ✅ |
| Letta | not published | agent loop incl LLM | ✅ |
| **Synapse** | **0.7ms cached / ~5ms cold** | pure retrieval, embedded | ❌ |

mem0/Letta latency is dominated by LLM cost. Synapse is the **storage-layer floor** — anyone can put an LLM in front of it and match mem0's pipeline while keeping the floor.

---

## Section 6 — Roadmap to Beat Both

### Quick Wins (1-3 days each)
- **Q1. Memory blocks (Letta-style core/archival split)** — typed `block` table + MCP edit tools. WHY: closes the #1 Letta moat. BENCH: LongMemEval session-level recall.
- **Q2. Tool-surface memory ops** — already MCP, add `add_memory`/`search_memory`/`edit_block` as first-class tools. WHY: becomes drop-in for any agent runtime.
- **Q3. Public LoCoMo + LongMemEval run** — reproduce mem0's table on Synapse storage. WHY: kills the "but mem0 has numbers" objection.

### Medium (1-2 weeks each)
- **M1. Optional LLM extractor pipeline** — pluggable Ollama/Phi-4-mini extract on write, store raw + extracted facts. WHY: matches mem0 v3 ADD-only semantics. BENCH: LoCoMo F1.
- **M2. Entity linking + boost** — NER pass, entity index, join-boost in T1. WHY: mem0's entity feature is their v3 win. BENCH: +5pt LoCoMo.
- **M3. Hierarchical memory tier** — in-context core block (LRU eviction → archival). WHY: full MemGPT parity. BENCH: long-context dialog tasks.

### Strategic (1-3 months)
- **S1. Reflection/consolidation daemon** — scheduled LLM compaction of old memories into summaries (Letta self-edit). WHY: closes the only architectural hole vs Letta. BENCH: 1M-message synthetic LoCoMo.
- **S2. Embedded property-graph layer** — sqlite-graph extension or custom edge-table. WHY: matches mem0 graph option without Neo4j ops. BENCH: multi-hop QA.
- **S3. Stateful agent abstraction** — `agent_state` + lifecycle on top of memory. WHY: makes Synapse a Letta drop-in, not just a memory layer. BENCH: agent-bench eval.

---

## Section 7 — The Final Pitch

**Synapse is the embedded sub-millisecond memory floor that mem0 and Letta need but don't have.** mem0 ships LLM-driven fact extraction with a 880ms server hop; Letta ships hierarchical memory blocks behind a Postgres server — Synapse ships a single signed CRDT-replicated SQLite file at 0.7ms p50, and both of them could run *on top* of it. The roadmap is not to replace their LLM pipelines — it's to be the storage substrate every agent framework in 2026 cannot afford to rebuild.
