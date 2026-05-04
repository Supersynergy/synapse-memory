# Tasks — sota-memory-upgrade

## Phase 0 — Mining (DONE 2026-04-29)
- [x] ghgrep `rust hnsw` / `rrf` / `cross encoder rerank rust` — patterns + LOC budget
- [x] Document → `docs/MINING-SOTA-2026-04-29.md`

## Phase 1 — ThinkRich roadmap (DONE)
- [x] Design tier matrix → `docs/SOTA-ROADMAP-2026-04-29.md`
- [x] Risk register
- [x] Architecture diff diagram

## Phase 2 — Tier-1 implementation
- [x] **T1.1** Typed memory schema (`memories`, `entities`, `memory_edges`, `extraction_queue`)
- [x] **T1.2** `MemoryType::default_weight()` (Fact 1.20 → Episodic 0.95, OMEGA-style)
- [x] **T1.6** `rrf_typed()` weighted RRF with `k=60` (Haystack/MongoDB default)
- [x] **T1.7** `synapse-rerank` crate, `Reranker` trait, `IdentityReranker`, `OnnxCrossEncoder` (fastembed JINA-rerank-v2)
- [x] **T1.8** `synapse-extract` crate, `Extractor` trait, `RuleExtractor`, `MlxExtractor` stub, queue ops
- [x] **T1.5** `Store::recall(RecallParams) -> Vec<RecallHit>` — fuse vec+FTS+entity+heat, pluggable reranker (DONE 2026-04-29)
- [x] **T1.4** Entity 1-hop expansion in recall pipeline (DONE 2026-04-29)
- [x] `Store::sota_migrate()` convenience added (DONE 2026-04-29)
- [ ] Auto-call `sota_migrate` from `Store::open` (~0.25 d)

## Phase 3 — Tier-2
- [ ] **T1.3** MLX subprocess wiring (real smollm2-1.7B-Instruct-4bit invocation) (~1 d)
- [x] **T2.6** Temporal parser crate `synapse-temporal` (chrono-english wrapper, DE+EN+Q-quarter, 4 tests) (DONE 2026-04-29)
- [ ] Wire `synapse-temporal::parse_temporal()` into `Store::recall` for period filter (~0.25 d)
- [ ] **T2.7** Lifecycle daemon (evolve/compact/decay) via launchd (~3 d)
- [ ] **T2.8** MemFS git mirror on `synapse-wal` (~4 d)

## Phase 4 — Bench harness
- [x] LongMemEval-S adapter skeleton (`bench/longmemeval/longmemeval_adapter.rs`, 2 tests)
- [ ] **T2.9** Download LongMemEval-S data (~2 GB) → `bench/longmemeval/data/`
- [ ] Run baseline (RuleExtractor + IdentityReranker)
- [ ] Run with OnnxCrossEncoder
- [ ] Run with MLX extractor
- [ ] Publish `docs/LONGMEMEVAL-RESULTS-<date>.md`

## Phase 4.5 — 99 % push (2026-04-29 session, latest-Rust port-from-analog)
- [x] Multi-hop entity graph traversal (BFS depth=2 default, 0.6^hop attenuation) — `sota::multi_hop_neighbors` + recall path. spec: `specs/multi-hop-graph/spec.md`. Source mining: petgraph::visit::Bfs pattern.
- [x] `RecallParams::max_hops` field (default 2)
- [x] Query decomposition (`PipelineHooks::decompose`, cue-word fallback) — `sota_pipeline.rs`. spec: `specs/query-decomp/spec.md`. Mined from langchain MultiQueryRetriever.
- [x] Self-RAG grading (`PipelineHooks::grade`, token-overlap fallback). spec: `specs/self-rag/spec.md`. Mined from Asai 2023 prompt.
- [x] HyDE rescue (`PipelineHooks::hyde`, prompt-template fallback). spec: `specs/hyde/spec.md`. Mined from Gao 2022 + llamaindex HyDEQueryTransform.
- [x] Lightweight NER: gazetteer (aho-corasick) + regex tier (email/url/iso-date/capitalised). `sota_ner.rs` (~155 LOC).
- [x] Evolve on ingest (`sota_pipeline::evolve_on_ingest`, EvolveCfg lo=0.55 hi=0.95). Mined from mem0/letta consolidation.
- [x] Compact nightly (`sota_pipeline::compact`, Jaccard ≥0.7 union-find clustering)
- [x] Extractor trait extended: `summarize` / `merge` / `decompose_query` / `grade_relevance` / `hyde` (default deterministic fallbacks)
- [x] `cargo test -p synapse-core -p synapse-extract -p synapse-temporal -p synapse-rerank` → **51 pass** (was 43, +8)
- [ ] Wire `synapse-learn::calibrate` Platt/isotonic into final RecallHit score (~0.5 d, primitives already exist)
- [ ] Promote `pipeline_recall` to `Store::pipeline_recall(hooks, params, …)` (currently free fn)
- [ ] MlxExtractor real subprocess (smollm2-1.7B JSON) — gating quality jump from ~95 → 98+
- [ ] Run LongMemEval-S — gating actual numeric SOTA score

## Phase 5 — Release
- [ ] All Phase 2 + Phase 4 done
- [ ] `cargo test --workspace` green
- [ ] `verification-loop` skill PASS
- [ ] Update `CHANGELOG.md`
- [ ] Tag `v0.4.0-sota`
