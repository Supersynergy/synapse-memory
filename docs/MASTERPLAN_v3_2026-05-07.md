# 🎯 Synapse Masterplan v3 — Surreal-Killer + Top10 Crown
**Datum**: 2026-05-07 · **Branch**: turbo-ndarray-fastpath → main eligible · **Hardware**: M4 Max

---

## Status Heute (Baseline)

| Metric | Aktuell | Tier |
|--------|--------:|------|
| Vec p50 | 0.06ms | 🥇 #1 industry |
| Ping QPS | 75k/s persistent | 🥇 #1 |
| FTS direct | 30µs apsw | 🥇 #1 |
| SQL COUNT direct | 27µs | 🥇 #1 |
| Hybrid cached | 2.32ms | 🥇 #1 |
| Boot | 6s sidecar persist | ✅ |
| Synapse #1 in 9-13/20 categories | matrix verified | strong |

---

## Phase 1 — "Sofort-3" Quick Wins (W1, ~2 Tage)

### Track 1.1 — INT8 quant default für daemon
- **Source**: `usearch::ScalarKind::I8` (already supported)
- **Change**: env-var `SYNAPSE_QUANT=i8|f16|f32`, daemon picks at index-build
- **Effort**: 30 LOC + rebuild (1 day)
- **Expected**: 4× memory @ R≥0.95, +5-10% QPS

### Track 1.2 — Matryoshka server-side
- **Source**: HuggingFace sentence-transformers `matryoshka.py`
- **Change**: daemon `Embed` op accepts optional `dim`, truncates+renorms (already in synxlib client)
- **Effort**: 20 LOC + rebuild (2-4h)
- **Expected**: 4× cache memory, 4× HNSW build at <1% recall loss

### Track 1.3 — INT8 packed cache in synxlib
- **Source**: novel — quantize f32 → int8 before sqlite-blob store
- **Change**: 30 LOC in `embed_cached`/`embed_cached_q8`
- **Effort**: 4h
- **Expected**: 4× cache compression (1.5KB → 384B per vec)

**Phase 1 Total**: 2 Tage · 4× memory savings everywhere · zero recall regression

---

## Phase 2 — "Surreal Parity" (W2-W4, ~3 Wochen)

### Track 2.1 — Graph Edges Layer (1w)
- **Source**: ogham-mcp pattern + petgraph algos + Synapse novel additions
- **Crate**: new `synapse-graph` mit edges table + traverse_n_hops + shortest_path
- **Already prototyped**: `~/projects/synapse/integrations/python/synapse_graph.py` (committed today)
- **Effort**: 200 LOC Rust port, 1w
- **Expected**: SurrealDB RELATE feature parity — multi-hop in <5ms via SQLite recursive CTE

### Track 2.2 — Live Queries via WebSocket (1w)
- **Source**: fastwebsockets (Deno-bench fastest, 250 hits) + tokio::sync::broadcast
- **Op**: new `LiveQuery` daemon op, axum-WS endpoint :9091
- **Pattern**: subscribe(filter) → broadcast on Put → push delta
- **Effort**: 300 LOC Rust + 50 LOC synxlib client
- **Expected**: <5ms push latency, 1000+ concurrent subscribers

### Track 2.3 — TX BEGIN/COMMIT API (3d)
- **Source**: SQLite native + rusqlite::Transaction
- **Op**: `Transaction { ops: [Put|Sql|Merge] }` atomic batch
- **Effort**: 100 LOC
- **Expected**: ACID multi-op, all-or-nothing

### Track 2.4 — Permissions/RBAC (1w)
- **Source**: PG row-level security pattern + JWT validation
- **Schema**: `auth_users`, `auth_roles`, `acl` table; check at query time
- **Effort**: 250 LOC
- **Expected**: scope-token-based read/write/admin tiers

**Phase 2 Total**: 3 Wochen · Surreal-Parität minus SurrealQL DSL · Synapse 1.5×+ schneller in jeder Surreal-shared category

---

## Phase 3 — "Crown Defense" (W5-W8, ~4 Wochen)

### Track 3.1 — RaBitQ Stage-0 default (1w)
- **Source**: FAISS `IndexRaBitQFastScan` SIMD pattern (port to Rust+NEON)
- **Already done**: T6 `rabitq.rs` proper-pattern (+42% recall)
- **Wire**: cascade `pack_signs_rotated_proper` als Stage-0 default in search.rs
- **Effort**: 200 LOC port + 50 LOC integration
- **Expected**: 32× memory compression default, R≥0.95 stage-0 prefilter

### Track 3.2 — Filtered HNSW pre (ACORN port) (2w)
- **Source**: RuVector ADR-160 (mining-first, novel adapt)
- **Pattern**: HNSW search während Filter applied auf graph traversal
- **Effort**: 400 LOC Rust + tests
- **Expected**: 2-10× faster filtered queries vs post-filter

### Track 3.3 — DiskANN streaming index (2w)
- **Source**: Milvus `VectorDiskIndex` + microsoft/DiskANN
- **Crate**: extend `synapse-quant` mit Vamana SSD-resident path
- **Effort**: 600 LOC port (heavy)
- **Expected**: 4GB RAM @ 1B vectors, single-node billion-scale

### Track 3.4 — IVF-PQ in synapse-quant (1w wire)
- **Source**: FAISS IndexIVFRaBitQ + scaffold synapse-quant exists
- **Effort**: 300 LOC Rust
- **Expected**: 8× memory compression, 50M→400M single-node

**Phase 3 Total**: 4 Wochen · Synapse #1 in 17-19/20 Kategorien · billion-scale single-node ready

---

## Phase 4 — "Production SaaS Ready" (W9-W12, ~4 Wochen)

### Track 4.1 — Multi-Tenant via brain.db ATTACH (1w)
- **Pattern**: per-tenant brain-{tenant}.db, daemon ATTACH at session-start
- **Schema**: tenant routing table + JWT scope check
- **Effort**: 200 LOC
- **Expected**: 100+ tenants single-binary, isolated

### Track 4.2 — libsql replication swap (2w)
- **Source**: Turso libsql (drop-in sqlite-compat with WAL streaming)
- **Migrate**: rusqlite → libsql_rusqlite (compat-API)
- **Effort**: 300 LOC + migration tests
- **Expected**: read-replica fan-out, multi-region eventual-consistency

### Track 4.3 — Pingora edge cache layer (3d)
- **Source**: Pingora cache module + LRU benches
- **Adapt**: synapse-edge erweitern um L1 cache vor upstream daemon
- **Effort**: 150 LOC
- **Expected**: 99%+ cache hit-rate edge, <1ms cached responses

### Track 4.4 — Cloud-deploy templates (3d)
- **Sources**: Railway/Coolify/Fly.io templates
- **Files**: `deploy/railway.toml`, `deploy/coolify.yml`, Dockerfile.alpine
- **Effort**: 50 LOC config
- **Expected**: 1-click-deploy, $10-30/mo per-tenant

**Phase 4 Total**: 4 Wochen · SaaS-tier ready · enterprise checkbox complete

---

## Phase 5 — "Differentiation Killers" (W13-W16, ~4 Wochen)

### Track 5.1 — ColbertReranker ONNX (1w)
- **Source**: Liquid AI LFM2-ColBERT-350M (newest 2026 multilingual)
- **Crate**: synapse-rerank ONNX scaffold exists
- **Effort**: 200 LOC + ONNX model fetch
- **Expected**: best-in-class multilingual rerank, +5-15% NDCG@10

### Track 5.2 — Geo Index (3d)
- **Source**: SpatiaLite extension + R-tree built-in SQLite
- **Schema**: `geo_index` table + ST_Distance helpers
- **Effort**: 100 LOC
- **Expected**: location-based queries, R-tree-fast

### Track 5.3 — Time-Series Functions (3d)
- **Source**: SQLite window-functions native + synapse-temporal exists
- **Wire**: synxlib helpers `synapse_lag`, `synapse_rolling_avg`
- **Effort**: 80 LOC
- **Expected**: trend-analysis built-in

### Track 5.4 — Schemafull/Schemaless mix (3d)
- **Source**: SQLite TYPES + JSON column native
- **Helper**: synxlib `define_schema(table, fields)` + `put_freeform`
- **Effort**: 100 LOC
- **Expected**: best-of-both flexibility

### Track 5.5 — synapseQL syntax sugar (2w)
- **Pattern**: SQL preprocessor — `RELATE x->rel->y` → `INSERT INTO edges`
- **Crate**: new `synapse-ql` parser (small antlr-rs grammar)
- **Effort**: 500 LOC
- **Expected**: Surreal-DX parity, no lock-in (still runs on SQLite)

**Phase 5 Total**: 4 Wochen · all 12 Surreal-features matched + 4 Synapse-unique adds

---

## 📊 Erwartete Resultate per Phase

| Phase | Time | Wins | New Synapse #1 categories |
|-------|------|------|---------------------------|
| **P1** | 2d | 4× memory · INT8 · Matryoshka · packed cache | +0 (consolidate existing) |
| **P2** | 3w | Graph · LiveQuery · TX · Permissions | +4 → **13-17 / 20** |
| **P3** | 4w | RaBitQ default · Filtered HNSW · DiskANN · IVF-PQ | +3 → **16-20 / 20** |
| **P4** | 4w | Multi-tenant · libsql · Edge cache · Cloud-deploy | +1 SaaS-tier |
| **P5** | 4w | ColBERT · Geo · TS · Schemafull · synapseQL | +2 multilingual+geo |

**Total**: 15 Wochen → **#1 in 18-20 / 20 categories** + Surreal-Parität + 1B-scale ready + SaaS-deployable

---

## 🎯 Erwartete Bench-Results (Phase 5 done)

| Op | Heute | P5 done | Speedup |
|----|------:|--------:|--------:|
| Vec p50 | 0.06ms | 0.04ms (RaBitQ stage-0) | 1.5× |
| Vec QPS | 16,667 | 50,000 | 3× |
| Hybrid cached | 2.32ms | 1.5ms (filtered HNSW) | 1.5× |
| FTS5 direct | 30µs | 30µs | gleich |
| Embed (server) | 1.18ms | 0.5ms (MLX-bf16-tuned + INT8) | 2× |
| Embed cached hit | 67µs | 67µs | gleich |
| **Boot** | 6s | 1s (sidecar warm + libsql) | 6× |
| **RAM @ 1M docs** | 1GB est | **256MB** (RaBitQ + INT8) | 4× |
| **Max scale** | 50M single-node | **1B single-node** | 20× |
| **Concurrent 8t** | 75k/s | 200k/s | 2.7× |

---

## 🏆 Erwartete Marktposition (P5 done)

| Domain | Synapse Position |
|--------|------------------|
| Local-first / DACH-Mittelstand | 🥇 #1 unangefochten |
| Embedded vec-DB | 🥇 #1 |
| Hybrid (vec+FTS+graph+TS+Geo) | 🥇 #1 unique combo |
| Single-binary multimodel | 🥇 #1 |
| MLX/Apple-Silicon native | 🥇 #1 only |
| CRDT branches | 🥇 #1 only |
| Doc-signing default | 🥇 #1 only |
| Mid-scale RAG (≤100M) | 🥇 #1 |
| Billion-scale single-node | 🥇 #1 (after P3) |
| Cloud SaaS | 🥈 #2 (after P4 templates) |
| Cross-encoder rerank | 🥇 (ColBERT+LightGBM) |

**Cannot compete**:
- Pinecone unbounded SaaS-tier (no plans to)
- Multi-region distributed billion-scale (Milvus/Vespa enterprise)

---

## 💰 €-Impact Schätzung

| Customer-Profile | Heute | P5 done |
|------------------|-------|---------|
| Mittelstand-DACH ≤10M docs | ✅ ready | ✅ |
| Agent-memory drop-in (Mem0-killer) | ✅ ready | ✅ + ColBERT-multilingual |
| RAG-pipeline (LangChain integration) | 🟡 manual wire | ✅ official adapter |
| Multi-tenant SaaS | 🔴 needs work | ✅ P4 done |
| Enterprise compliance (DSGVO) | ✅ ready | ✅ + Permissions P2 |
| Billion-scale recommendation | 🔴 | ✅ P3 done |

**Pricing model** (selbst-host vs cloud):
- OSS Free: heute + alle P-tracks
- Cloud Tier 1: $10-30/mo Mittelstand (P4 templates)
- Cloud Tier 2: $100-300/mo enterprise multi-tenant
- Self-host Pro: licence ed25519 (already implemented in synapse-license crate)

---

## 🔧 Effort-Summary

| Phase | Wochen | LOC | Rebuild needed |
|-------|--------|-----|----------------|
| P1 | 0.5 | 80 | 1× cargo |
| P2 | 3 | ~750 | 2× cargo |
| P3 | 4 | ~1500 | 3× cargo |
| P4 | 4 | ~700 | 2× cargo |
| P5 | 4 | ~1000 | 2× cargo |
| **Total** | **15-16w** | **~4000 LOC** | **10 rebuilds** |

Solo-dev pace 1 Person · 2 dev parallel = ~8 Wochen.

---

## 🚦 Risk + Mitigation

| Risk | Mitigation |
|------|-----------|
| usearch INT8 recall regression | bench all 50/100k pre-deploy |
| Matryoshka quality drop | A/B test recall@10 before flip |
| RaBitQ stage-0 false-prefilter | f16 rerank-stage catches |
| LiveQuery socket-storm | tokio::broadcast lagged-receiver-drop |
| libsql migration drift | feature-flag both paths during transition |
| Pingora-edge upstream-dependency | cache-layer absorbs daemon-down |

---

## 📋 Nächster konkreter Schritt

**Heute**: Phase 1 Track 1.2 (Matryoshka server-side) — 2-4h, kleinste Risiko, größter immediate-win

**Nach P1 done**: Phase 2 Track 2.1 (Graph layer Rust port from prototype)

**Strategic checkpoint nach P3**: Re-bench 20×20 matrix, decide P4-cloud-tier or P5-killer-features first.
