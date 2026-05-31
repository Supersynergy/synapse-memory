# 00 — Synapse State-of-the-Repo Audit (Ground Truth)

> Honest, blunt inventory of the Synapse monorepo as the basis for the
> `synapse-db` / `synapse-memory` split. No inflation: stubs are called stubs,
> "REAL" means it has substantive implementation (meaningful LOC, public API,
> zero `todo!`/`unimplemented!`).
>
> Repo: `/Users/master/projects/synapse` · 55 cargo workspace members ·
> `target/` = 263G (gitignored). One monorepo, two product identities.

---

## 1. Executive Summary

The repo is **one monorepo carrying two products plus one vertical spinoff**,
which is exactly why `SPEC.md` and `ARCHITECTURE.md` disagree (see §4).

**Crate census (55 workspace members):**

| Classification | Count | Meaning |
|---|---|---|
| **REAL** (substantive, shippable, 0 todo/unimpl) | **~38** | Real code, real public API |
| **Partial** (works but linux-only / narrow / WIP surface) | **~6** | `iouring`, `jit`, `tsdb`, `cluster`, `mysql`, `colbert`-adjacent |
| **Stub / scaffold / dead** | **~7** | `raft`, `wal`, `seg`, `e2e`, `vlog`, `ultra`(dup), `lib-demo` |
| Language bindings / thin | ~4 | `py`, `js`, `metal`, `embed-gpu` |

**The foundation is real.** `synapse-core` is 14,612 LOC / 322 pub fns / 96 pub
structs — it is THE foundation and everything points at it. The DB surfaces
(`synapsql`, `synapse-server`, wire protocols) and the memory surfaces
(`space`, `extract`, `rerank`, `learn`, `synapsed`, `cli`, `mcp`) are both
real and both substantial.

**Only one true dependency smell:** `synapse-mcp -> synapse-market`. MCP is a
memory-product surface; it must NOT pull the trading/market vertical. This edge
gets **cut** in the split.

**Verified invariant:** no DB-bucket crate depends on any memory crate
(`space`/`extract`/`rerank`/`temporal`). `tune` is consumed by `server`, so it
is DB-side, not memory. The DAG supports the proposed split cleanly.

**Three target repos:**
- `synapse-db` — the engine product (shared foundation + DB/SQL-wire/storage surfaces).
- `synapse-memory` — the agent-memory product (depends on `synapse-db` foundation via submodule + path deps).
- `synapse-market` — the trading vertical spinoff (own repo; `mcp->market` edge cut).

---

## 2. Per-Crate Table (grouped by target bucket)

Status legend: **REAL** = substantive + 0 todo/unimpl · **partial** = works but
narrow/WIP/platform-bound · **stub** = scaffold/empty/placeholder · **dup** =
duplicate of another crate · **thin** = binding/glue.

### 2a. SHARED FOUNDATION (lives in `synapse-db`; `synapse-memory` depends on these)

| Crate | Bucket | Status | LOC | pub_fns | Notes |
|---|---|---|---:|---:|---|
| synapse-core | shared | **REAL** | 14612 | 322 | THE foundation; 96 pub structs; 1 todo marker only |
| synapse-kernel | shared | **REAL** | 567 | 16 | SIMD kernels (SimSIMD); 1 pub struct; note: per-pkg profile warning |
| synapse-engine | shared | **REAL** | 185 | 4 | small but real glue used by core consumers |
| synapse-ann | shared | **REAL** | 1339 | 18 | ANN index |
| synapse-fts | shared | **REAL** | 183 | 6 | FTS5-style full-text |
| synapse-graph | shared | **REAL** | 2269 | 49 | graph layer; 14 pub structs |
| synapse-quant | shared | **REAL** | 623 | 14 | quantization (MRL/int8/1-bit) |
| synapse-spann | shared | **REAL** | 344 | 15 | SPANN index (deps kernel only) |
| synapse-obs | shared | **REAL** | 164 | — | observability |

### 2b. `synapse-db` (engine product = foundation + these surfaces)

| Crate | Bucket | Status | LOC | pub_fns | Notes |
|---|---|---|---:|---:|---|
| synapsql | db | **REAL** | 2701 | 50 | SQL surface; deps core/libsql/mysql/pg |
| synapse-libsql | db | **REAL** | 600 | — | libSQL integration |
| synapse-mysql | db | partial | 1075/2202 | — | MySQL wire; partial coverage |
| synapse-pg | db | **REAL** | — | — | Postgres wire; deps libsql |
| synapse-server | db | **REAL** | 532 | — | server entry; deps auth/graph/libsql/mysql/ops/pg/tune |
| synapse-auth | db | **REAL** | 151 | 10 | 4 pub structs |
| synapse-ops | db | **REAL** | 169 | — | ops surface |
| synapse-cms | db | **REAL** | 224 | — | CMS surface on core |
| synapse-olap | db | **REAL** | 92 | — | OLAP |
| synapse-mlx-olap | db | **REAL** | 316 | — | MLX-accelerated OLAP |
| synapse-tsdb | db | partial | 881 | 14 | time-series; WIP surface |
| synapse-jit | db | partial | 1285 | 20 | JIT; partial (11 pub structs) |
| synapse-iouring | db | partial | 1387 | 25 | linux io_uring only |
| synapse-ring | db | **REAL** | 250 | — | ring buffer |
| synapse-cluster | db | partial | 1539 | 34 | distributed; partial |
| synapse-raft | db | **stub** | 77 | 0 | **STUB** — 0 pub fns, 4 pub structs; scaffold only |
| synapse-tier | db | **REAL** | 147 | 1 | tiering; 4 pub structs |
| synapse-stream | db | **REAL** | 485 | — | streaming |
| synapse-tune | db | **REAL** | 896 | 29 | autotune; consumed by server (DB-side, not memory) |
| synapse-migrate | db | **REAL** | 1011 | — | migrations |
| synapse-license | db | **REAL** | 501 | — | licensing |
| msql-srv-patched | db | thin | — | — | patched MySQL server wire dep |
| synapse-mysql-async | db | thin | — | — | async MySQL dep |

### 2c. `synapse-memory` (agent-memory product; depends on `synapse-db`)

| Crate | Bucket | Status | LOC | pub_fns | Notes |
|---|---|---|---:|---:|---|
| synapse-space | memory | **REAL** | 1161 | 16 | core memory space; deps core/ann/engine/rerank |
| synapse-extract | memory | **REAL** | 995 | 12 | extraction; deps core/graph |
| synapse-rerank | memory | **REAL** | 596 | 12 | rerank; deps core |
| synapse-temporal | memory | **REAL** | 152 | 3 | temporal; standalone (2 pub structs) |
| synapse-learn | memory | **REAL** | 1001 | 37 | learning; deps core |
| synapsed | memory | **REAL** | 2805 | 15 | daemon; deps core/license/rerank |
| synapse-cli | memory | **REAL** | 2303 | — | CLI; deps core/graph/learn |
| synapse-mcp | memory | **REAL** | 1113 | — | MCP server — **CUT `->market` dep** |
| synapse-colbert | memory | **REAL** | 1216 | 24 | late-interaction; deps kernel only (could go shared) |
| synapse-splade | memory | **REAL** | 721 | — | neural-sparse; recall-quality |
| synapse-fusion | memory | **REAL** | 277 | — | deps colbert/splade |
| synapse-multimodal | memory | **REAL** | 543 | — | multimodal |
| synapse-media | memory | **REAL** | 1339 | 26 | media; 9 pub structs |
| synapse-metal | memory | thin | 93 | — | Metal accel binding |
| synapse-embed-gpu | memory | thin | 85 | — | GPU embed binding |
| synapse-py | memory | thin | 392 | 0 | Python binding; 7 pub structs, 0 pub fns |
| synapse-js | memory | thin | 110 | — | JS binding |

> Note: `colbert`/`splade`/`fusion` only depend on `kernel`, so they *could*
> live in the shared foundation. Keep them in `memory` (they are recall-quality
> rerankers) unless the DB product later needs neural rerank.

### 2d. VERTICAL spinoff (own repo `synapse-market`)

| Crate | Bucket | Status | LOC | pub_fns | Notes |
|---|---|---|---:|---:|---|
| synapse-market | market | **REAL** | 7389 | 209 | trading vertical; 72 pub structs; deps core/tsdb |
| synapse-market-py | market | thin | — | — | python binding; deps market |
| synapse-market-ts | market | thin | — | — | ts binding; deps market |

### 2e. CUT / scaffold (archive or roadmap-only)

| Crate | Bucket | Status | LOC | Notes |
|---|---|---|---:|---|
| synapse-ultra | cut | **dup** | 2626 | duplicate "synapsestore" daemon; merge into synapsed or archive |
| synapse-vlog | cut | stub | 225 | versioned-log scaffold |
| synapse-wal | cut | **STUB** | 13 | 13 LOC — empty scaffold |
| synapse-seg | cut | **STUB** | 13 | 13 LOC — empty scaffold |
| synapse-e2e | cut | **EMPTY** | 1 | 1 LOC — rebuild from scratch |
| synapse-lib-demo | cut | demo | — | demo crate, not a product |

---

## 3. Compile Truth

`cargo check --workspace` (warm, captured this session) — **CLEAN.**

```
Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.35s
```

- **0 compile errors. 0 crate failures.** All 55 workspace members type-check.
- One **non-fatal warning** only: per-package `[profile]` in
  `crates/synapse-kernel/Cargo.toml` is ignored because profiles must be set at
  the workspace root. Cosmetic; should be moved to the root `Cargo.toml` during
  the split.

Caveat: this was a warm incremental `cargo check` (1.35s). It proves the type
graph is consistent. It does NOT run tests, clippy, or a release build — those
gates run during the physical split, not in this audit.

---

## 4. The SPEC.md ↔ ARCHITECTURE.md Contradiction

There is a real, documented contradiction in the repo today:

- **`SPEC.md` (2026-05-25)** says: "15 active crates"; distributed,
  SQL-wire, and multimodal are **OUT of scope**; mission = "Context-OS".
- **`ARCHITECTURE.md`** lists **~50 crates** including the distributed layer
  (cluster/raft) and the SQL-wire protocols (mysql/pg/libsql).

These are not both wrong — they are describing **two different products that
currently share one repo**:

- `SPEC.md` was (unknowingly) describing the **`synapse-memory`** product:
  ~15 memory-side crates, Context-OS mission, no SQL-wire, no distributed.
- `ARCHITECTURE.md` was describing the **whole monorepo** = `synapse-db`
  (foundation + SQL-wire + distributed + storage) **plus** `synapse-memory`.

**The split resolves the contradiction structurally:** after the split, each
repo gets its own `SPEC.md` + `ARCHITECTURE.md` that finally match.
`synapse-memory/SPEC.md` becomes the current Context-OS SPEC (and is now
correct about its ~17 crates); `synapse-db/ARCHITECTURE.md` owns the engine,
SQL-wire, and distributed story. No more single doc trying to describe two
products with opposite scopes.

---

## 5. Dead / Scaffold to Cut

Cut or quarantine before the split so neither product inherits clutter:

**Crate-level (archive or move to roadmap-only):**

- `synapse-wal` (13 LOC) — empty scaffold. **Cut.**
- `synapse-seg` (13 LOC) — empty scaffold. **Cut.**
- `synapse-e2e` (1 LOC) — empty. **Rebuild** real e2e tests post-split.
- `synapse-raft` (77 LOC, **0 pub fns**) — stub; keep as roadmap-only in
  `synapse-db` or archive until the distributed story is real.
- `synapse-vlog` (225 LOC) — versioned-log scaffold; archive.
- `synapse-ultra` (2626 LOC) — **duplicate** "synapsestore" daemon; merge the
  unique bits into `synapsed` or archive the whole crate.
- `synapse-lib-demo` — demo, not a product; archive.

**Directory / doc clutter:**

- `synapsestore/` — legacy duplicate crates (wal/seg/ultra). **Archive.**
- `products/` — a prior half-done split attempt (agentdb-local, benchmark-kit,
  claude-code-memory, enterprise-onprem, freshness-router). Reuse the
  *naming/ideas*, but the crate-level split here **supersedes** it.
  `products/release/context-os/` is the clean `synapse-memory` release slice —
  **reuse** as the memory product's release dir.
- **17 root `*.md`** — KEEP: ARCHITECTURE, SPEC, README, CHANGELOG, CONTRIBUTING,
  LICENSE-CORE, LICENSE-ENGINE (dual-license). ARCHIVE to `docs/archive/`:
  BENCH_2026-05-10, PROD_READY_2026-05-10, PUBLISH_STATUS_2026-05-13,
  CORRECTIVE-ACTION-PLAN-2026-04-25, LAUNCH_CHECKLIST_v1.0.1,
  RELEASE_NOTES_v1.0.1-rc{,.1}, HN_LAUNCH, KNOWN-ISSUES, tuning.md.
- `docs/` (~90 files, many dated bench/plan) → sweep into `docs/archive/`.
- `examples/` (3.2G) — almost certainly built `target/` dirs and/or committed
  data inside examples. Add `examples/**/target` to `.gitignore` and check for
  committed `*.db`/`*.parquet` before either repo inherits it.

**CI (11 workflows in `.github/workflows`):**

- Consolidate the overlapping trio: `ci.yml` vs `rust-ci.yml` vs `quality.yml`.
- Move `synapse-market-ci.yml` + `synapse-market.yml` to the **market spinoff**
  repo.
- Keep `bench`, `bench-nightly`, `linux-bench`, `docs`, `release`, `security`
  and split them per target repo as appropriate.

**One dependency edge to cut (the only real smell):**

- `synapse-mcp -> synapse-market`. MCP is a memory surface; it must not pull the
  trading vertical. Sever this edge so `synapse-mcp` deps only the memory/foundation
  graph.
