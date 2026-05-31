# 02 — Architecture (Reconciled): The Two-Product World

> Status: plan-of-record for the synapse split. Date: 2026-05-29.
> Scope: one architecture doc describing the deliberate split of the
> `/Users/master/projects/synapse` monorepo into two product identities
> plus one vertical spinoff.

---

## 0. The SPEC ↔ ARCHITECTURE contradiction — resolved by the split

The repository currently ships two documents that appear to disagree:

| Doc | Claims | What it was *actually* describing |
|-----|--------|-----------------------------------|
| `SPEC.md` (2026-05-25) | "15 active crates"; distributed, SQL-wire, multimodal are **OUT of scope**; mission = **Context-OS / agent memory**. | The **synapse-memory** product (the agent-memory slice). |
| `ARCHITECTURE.md` | ~50 crates including distributed (`raft`, `cluster`, `tier`, `stream`), SQL-wire (`mysql`, `pg`, `synapsql`), OLAP, TSDB. | The **whole monorepo** = `synapse-db` + `synapse-memory`. |

These were never in conflict — they were describing **two different products living in one repo**. This split makes the boundary explicit:

- **`synapse-db`** — the engine product. "World's best embedded vector+SQL+graph database." Owns kernel, core store, query fusion, and all the SQL-wire / OLAP / TSDB / distributed surfaces that `SPEC.md` declared out of scope.
- **`synapse-memory`** — the agent-memory product. The Context-OS mission from `SPEC.md`. Built **on top of** `synapse-db`. This is the "15 active crates" world.
- **`synapse-market`** — a vertical spinoff (financial market data), moved to its own repo with the `mcp -> market` dependency cut.

One monorepo today; the DAG below justifies the cut lines. 55 cargo workspace members (incl. bench crates); `target/` (263G) is gitignored and not part of the split.

---

## 1. `synapse-db` — the engine product

`synapse-db` is a layered embedded database engine. It owns the **shared foundation** (Section 3) plus all the data-surface crates. It depends on nothing from the memory side.

### 1.1 Layers (bottom → top)

```
┌──────────────────────────────────────────────────────────────────────┐
│ L4  SURFACES                                                           │
│     SQL-wire (mysql / pg / synapsql) · OLAP · TSDB · distributed       │
│     server · auth · ops · cms · tier · stream · cluster · raft         │
├──────────────────────────────────────────────────────────────────────┤
│ L3  QUERY                                                              │
│     RRF fusion · ANN search · FTS · graph traversal · tune (autotune)  │
├──────────────────────────────────────────────────────────────────────┤
│ L2  CORE STORE                                                         │
│     SQLite + FTS5 + vector columns + graph adjacency  (synapse-core)   │
├──────────────────────────────────────────────────────────────────────┤
│ L1  KERNEL                                                             │
│     SIMD kernels (SimSIMD: 1-bit / int8 / MRL-128 / f16)               │
└──────────────────────────────────────────────────────────────────────┘
```

### 1.2 Maturity per layer (REAL vs partial vs roadmap)

| Layer | Crate(s) | Maturity | Evidence |
|-------|----------|----------|----------|
| L1 Kernel | `kernel` | **REAL** | 567 LOC, 16 pub fns, SIMD, 0 TODO |
| L2 Core store | `core` | **REAL** | 14,612 LOC, 322 pub fns, 96 structs — THE foundation |
| L3 Query | `ann`, `fts`, `graph`, `engine`, `quant`, `spann` | **REAL** | ann 1339, graph 2269, engine 185, quant 623, fts 183, spann 344 — all 0 TODO |
| L3 Autotune | `tune` | **REAL** | 896 LOC, 29 pub fns (used by `server` → DB-side, **not** memory) |
| L4 SQL-wire | `synapsql`, `libsql`, `mysql`, `pg` | **REAL** | synapsql 2701; mysql 1075/2202; pg deps libsql |
| L4 Server/admin | `server`, `auth`, `ops`, `cms`, `license`, `migrate` | **REAL / thin** | server 532, auth 151, ops 169, cms 224, license 501, migrate 1011 |
| L4 OLAP/TSDB | `olap`, `mlx-olap`, `tsdb`, `jit`, `iouring`, `ring` | **partial** | tsdb 881, jit 1285, iouring 1387 (linux), olap 92, mlx-olap 316, ring 250 |
| L4 Distributed | `cluster`, `tier`, `stream` | **partial** | cluster 1539, tier 147, stream 485 |
| L4 Distributed | `raft` | **ROADMAP STUB** | 77 LOC, 0 pub fns, 4 structs |
| (write path) | `wal`, `seg` | **ROADMAP STUB** | wal 13 LOC, seg 13 LOC |

**Roadmap stubs** (`raft`, `wal`, `seg`) ship as scaffolding only — documented as roadmap, not as working features. **Partial** layers (`cluster`, `tier`, `stream`, OLAP/TSDB family) work for the supported path but are not feature-complete; each must carry an explicit "supported / unsupported" note in its crate README.

### 1.3 Crate map

| Crate | Layer | Role | Internal deps |
|-------|-------|------|---------------|
| `synapse-kernel` | L1 | SIMD distance kernels | — (shared) |
| `synapse-core` | L2 | SQLite+FTS+vec+graph store | ann, engine, fts, graph, kernel, obs, quant, spann (shared) |
| `synapse-ann` | L3 | ANN index | core foundation (shared) |
| `synapse-fts` | L3 | full-text search | (shared) |
| `synapse-graph` | L3 | graph adjacency/traversal | (shared) |
| `synapse-engine` | L3 | query engine glue | (shared) |
| `synapse-quant` | L3 | quantization | (shared) |
| `synapse-spann` | L3 | SPANN large-scale ANN | kernel (shared) |
| `synapse-obs` | L3 | observability | (shared) |
| `synapse-tune` | L3 | autotuning | core |
| `synapsql` | L4 | SQL query layer | core, libsql, mysql, pg |
| `synapse-libsql` | L4 | libSQL backend | — |
| `synapse-mysql` | L4 | MySQL wire | libsql |
| `synapse-pg` | L4 | Postgres wire | libsql |
| `synapse-server` | L4 | network server | auth, graph, libsql, mysql, ops, pg, tune |
| `synapse-auth` | L4 | auth | (standalone) |
| `synapse-ops` | L4 | ops/admin | core |
| `synapse-cms` | L4 | content mgmt | core |
| `synapse-migrate` | L4 | migrations | core |
| `synapse-license` | L4 | licensing | (used by synapsed) |
| `synapse-olap` / `synapse-mlx-olap` | L4 | OLAP | (standalone) |
| `synapse-tsdb` | L4 | time-series | (standalone) |
| `synapse-jit` / `synapse-iouring` / `synapse-ring` | L4 | perf/IO backends | (standalone) |
| `synapse-cluster` / `synapse-tier` / `synapse-stream` | L4 | distributed | core (cluster) |
| `synapse-raft` | L4 | consensus (roadmap stub) | (standalone) |
| `synapse-wal` / `synapse-seg` | write-path | roadmap stubs | (standalone) |

### 1.4 Data flow — put / query

```
        ┌──────────────────────── synapse-db PUT path ─────────────────────┐
        │                                                                    │
 caller ──► surface (synapsql / mysql / pg / server)                         │
        │        │                                                           │
        │        ▼                                                           │
        │   synapse-core ─┬─► SQLite row write                              │
        │                 ├─► FTS5 index update           (synapse-fts)     │
        │                 ├─► vector column + quantize     (quant + kernel)  │
        │                 └─► graph adjacency upsert       (synapse-graph)   │
        └────────────────────────────────────────────────────────────────────┘

        ┌──────────────────────── synapse-db QUERY path ───────────────────┐
        │  query (vec | text | graph | hybrid)                              │
        │        │                                                           │
        │        ├─► ANN search        (ann/spann + kernel SIMD)             │
        │        ├─► FTS5 BM25         (fts)                                 │
        │        └─► graph walk        (graph)                              │
        │                 │                                                  │
        │                 ▼                                                  │
        │           RRF fusion (engine) ──► ranked candidate IDs            │
        │                 │                                                  │
        │                 ▼                                                  │
        │           SQLite row hydrate (core) ──► results                    │
        └────────────────────────────────────────────────────────────────────┘
```

`synapse-db`'s query layer **stops at RRF fusion** — it returns fused, ranked candidates. Neural reranking (ColBERT/SPLADE/fusion) is a **recall-quality** concern and lives in `synapse-memory` (Section 2).

---

## 2. `synapse-memory` — the agent-memory product

`synapse-memory` is the Context-OS product from `SPEC.md`. It is built **on top of** `synapse-db`: every store/query call goes through the shared foundation crates (Section 3). It adds the ingest, recall-quality, and agent-facing surfaces.

### 2.1 Layers (request → response)

```
┌──────────────────────────────────────────────────────────────────────┐
│  SURFACES   synx CLI · MCP server · Python · JS · daemon (synapsed)    │
├──────────────────────────────────────────────────────────────────────┤
│  RECALL     rerank · colbert · splade · fusion · temporal · learn      │
├──────────────────────────────────────────────────────────────────────┤
│  STORE      (delegated to synapse-db core/ann/fts/graph)               │
├──────────────────────────────────────────────────────────────────────┤
│  INGEST     extract (graph) · embed (metal / embed-gpu) · multimodal   │
├──────────────────────────────────────────────────────────────────────┤
│  ───────────────  synapse-db shared foundation  ─────────────────────  │
└──────────────────────────────────────────────────────────────────────┘
```

### 2.2 Crate map

| Crate | Layer | Role | Internal deps | Status |
|-------|-------|------|---------------|--------|
| `synapse-extract` | ingest | entity/relation extraction | core, graph | extract 995 LOC |
| `synapse-embed-gpu` | ingest | GPU embedding | (standalone) | 85 LOC |
| `synapse-metal` | ingest | Metal embedding | (standalone) | 93 LOC |
| `synapse-multimodal` | ingest | multimodal handling | (standalone) | 543 LOC |
| `synapse-media` | ingest | media pipeline | core | 1339 LOC |
| `synapse-space` | store/recall | memory spaces | core, ann, engine, rerank | 1161 LOC, REAL |
| `synapse-rerank` | recall | reranking | core | 596 LOC, REAL |
| `synapse-colbert` | recall | late-interaction rerank | kernel | 1216 LOC |
| `synapse-splade` | recall | neural-sparse rerank | — | 721 LOC |
| `synapse-fusion` | recall | colbert+splade fusion | colbert, splade | 277 LOC |
| `synapse-temporal` | recall | temporal/freshness | (standalone) | 152 LOC |
| `synapse-learn` | recall | online learning / feedback | core | 1001 LOC, REAL |
| `synapsed` | surface | daemon (`/tmp/synapse.sock`) | core, license, rerank | 2805 LOC, REAL |
| `synapse-cli` | surface | `synx` CLI | core, graph, learn | 2303 LOC, REAL |
| `synapse-mcp` | surface | MCP server | core (after cutting `market`) | 1113 LOC, REAL |
| `synapse-py` | surface | Python bindings | — | 392 LOC |
| `synapse-js` | surface | JS bindings | — | 110 LOC |

> **Note on colbert/splade/fusion:** these only depend on `kernel`, so they *could* live in the shared foundation. They are kept in `synapse-memory` because they are recall-quality concerns the engine product does not promise. Move them to shared only if `synapse-db` later needs neural rerank.

> **MCP smell fixed:** today `synapse-mcp -> synapse-market`. The split **cuts** this dependency — the market vertical does not belong in the agent-memory MCP surface. After the cut, `synapse-mcp` depends only on the foundation.

### 2.3 Data flow — recall

```
        ┌──────────────────────── synapse-memory RECALL ──────────────────┐
        │  agent query  (via synx CLI / MCP / py / js / synapsed)          │
        │        │                                                          │
        │        ▼                                                          │
        │   ┌── synapse-db foundation ──────────────────────────────┐      │
        │   │  ANN + FTS + graph ──► RRF fusion ──► candidate set    │      │
        │   └────────────────────────────────────────────────────────┘     │
        │        │  (top-K candidates)                                      │
        │        ▼                                                          │
        │   temporal freshness weighting   (temporal)                       │
        │        │                                                          │
        │        ▼                                                          │
        │   neural rerank:  colbert ─┐                                      │
        │                   splade ──┴─► fusion ──► rerank  (final order)   │
        │        │                                                          │
        │        ▼                                                          │
        │   learn: log feedback signal ──► online update    (learn)        │
        │        │                                                          │
        │        ▼                                                          │
        │   ranked memories ──► agent                                       │
        └────────────────────────────────────────────────────────────────────┘

   INGEST (write side):
        doc ──► extract (entities/relations) ──► embed (metal/embed-gpu)
            ──► synapse-db core PUT (store)  ──► graph linked
```

---

## 3. The shared-foundation boundary

The two products meet at a single, stable boundary: the **shared foundation** crates.

### 3.1 What is shared (lives in `synapse-db`, consumed by `synapse-memory`)

```
kernel · core · engine · ann · fts · graph · quant · spann · obs
```

These are the only crates `synapse-memory` may depend on from the engine side. The hard rule:

> **`synapse-db` MUST NOT depend on any memory crate.**
> Verified: no DB crate depends on `space` / `extract` / `rerank` / `temporal`.
> `tune` is consumed by `server` → `tune` is DB-side, **not** memory.

### 3.2 How `synapse-memory` consumes the foundation (recommended: Option A)

**Option A — git submodule + path deps (local-first, no registry).**
`synapse-db` is the primary repo. `synapse-memory` pulls the foundation crates via a git submodule at `vendor/synapse-db`, wired with Cargo **path** dependencies:

```toml
# synapse-memory/Cargo.toml  (workspace)
[patch.crates-io]
synapse-kernel = { path = "vendor/synapse-db/crates/kernel" }
synapse-core   = { path = "vendor/synapse-db/crates/core" }
# … ann, fts, graph, quant, spann, engine, obs
```

- Pros: zero registry infra; pin foundation by git SHA; dev with live path override.
- Submodule SHA is the contract; bump = explicit memory-side commit.

**Option B — private registry (alternative).** Publish the 9 foundation crates to a private cargo registry; `synapse-memory` depends on versioned releases. Use only if multiple external consumers need the foundation. More infra, stronger versioning.

### 3.3 Repository topology

```
synapse-db            (primary repo)  ── foundation + DB surfaces (L1–L4)
   └── consumed by ▼
synapse-memory        (repo #2)       ── vendor/synapse-db submodule + memory crates
synapse-market        (repo #3)       ── market vertical; mcp->market dep CUT
                                          (synapse-market, -py, -ts)
```

**Cut / archived (not shipped as product crates):** `synapse-wal`, `synapse-seg` (roadmap stubs), `synapse-e2e` (empty → rebuild), `synapse-ultra` (synapsestore duplicate daemon → merge into `synapsed` or archive), `synapse-vlog`, `synapse-lib-demo`, legacy `synapsestore/` and `products/` half-split.

---

## 4. Product promises — what each side owns vs. defers

| Concern | `synapse-db` promises | `synapse-memory` promises |
|---------|------------------------|----------------------------|
| **Mission** | World's best embedded vector + SQL + graph DB. | Context-OS: agent long-term memory. |
| **Storage** | Owns it: SQLite+FTS+vec+graph, durability, schema. | Defers to `synapse-db`. Never re-implements storage. |
| **Retrieval primitives** | ANN, FTS BM25, graph walk, **RRF fusion**. | Consumes them; does not re-implement. |
| **Recall quality** | Not promised. Stops at RRF fusion. | Owns it: temporal weighting, ColBERT/SPLADE/fusion rerank, online `learn`. |
| **Ingest / extraction** | Not promised. | Owns it: entity/relation extract, embedding, multimodal. |
| **SQL-wire / OLAP / TSDB** | Owns it (real + partial layers). | Out of scope (this is the `SPEC.md` "out of scope" list). |
| **Distributed** | Owns it (partial: cluster/tier/stream; roadmap: raft). | Out of scope. |
| **Agent surfaces** | Network server, SQL clients, admin. | `synx` CLI, MCP, Python/JS bindings, `synapsed` daemon. |
| **Market data** | No. | No. → lives in `synapse-market` spinoff. |

### 4.1 Direction of dependency (invariant)

```
   synapse-memory ───► synapse-db (foundation)        ✅ allowed
   synapse-db     ───► synapse-memory                  ❌ FORBIDDEN
   synapse-mcp    ───► synapse-market                  ❌ CUT in this split
```

This single arrow direction is the architectural contract that keeps "world's best database" and "agent memory" cleanly separable products from one codebase.

---

## 5. CI consolidation (follow-on)

The split also resolves CI sprawl (11 workflows today):

- **`synapse-db` repo:** consolidate `ci.yml` + `rust-ci.yml` + `quality.yml` into one `ci.yml`; keep `bench`, `bench-nightly`, `linux-bench`, `security`, `release`, `docs`.
- **`synapse-memory` repo:** inherit consolidated `ci.yml` shape; foundation built from `vendor/synapse-db` submodule.
- **`synapse-market` repo:** owns `synapse-market-ci.yml` + `synapse-market.yml` (moved out of the engine/memory repos).

Doc cleanup: `ARCHITECTURE`, `SPEC`, `README`, `CHANGELOG`, `CONTRIBUTING` stay; dated bench/plan/launch `*.md` → `docs/archive/`. Dual-license `LICENSE-CORE` / `LICENSE-ENGINE` retained.
