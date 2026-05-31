# Synapse Split — Executable Plan

> **Status:** plan (execute next in a git worktree). **Date:** 2026-05-29. **Repo:** `/Users/master/projects/synapse` (55 cargo workspace members incl. bench crates; `target/` = 263G, gitignored).
> **Goal:** split one monorepo with two product identities into **synapse-db** ("world's best database" engine) + **synapse-memory** (agent-memory product, depends on db) + a vertical spinoff **synapse-market**, every public function cleanly documented, each repo with its own CI.
> Ground-truth facts (DAG, per-crate LOC/status, bucket assignment) are pre-gathered and treated as authoritative below — verified against `crates/*` (51 dirs present), `.github/workflows/` (11 files), 17 root `*.md`, branch `main`.

---

## 1. The two products + the vertical

**synapse-db (the engine / "world's best database").** The foundation plus every storage, query, and serving surface: SIMD kernels, the core index/storage engine, ANN + FTS + graph retrieval, quantization, SPANN, observability — then the SQL-wire layer (`synapsql` + MySQL/Postgres/libSQL adapters), the network server, auth/ops/CMS, columnar/OLAP/TSDB analytics, JIT and io_uring/ring fast paths, clustering + raft + tiering + streaming, the autotuner, migrations, and licensing. This is a self-contained database product: it depends on nothing in the memory layer. It is the primary repo and the home of the shared foundation crates.

**synapse-memory (agent memory).** The Context-OS product that turns the engine into long-term agent memory: spaces, extraction, rerank, temporal reasoning, online learning, the daemon (`synapsed`), the CLI, the MCP server, language bindings (py/js), GPU/Metal embedding accel, and recall-quality crates (ColBERT late-interaction, SPLADE neural-sparse, fusion, multimodal, media). It **depends on** synapse-db's foundation crates (kernel/core/engine/ann/fts/graph/quant/spann/obs) and consumes them as a vendored submodule with Cargo path deps. This is the product `SPEC.md` (2026-05-25) actually describes — "15 active crates, distributed/SQL-wire OUT of scope, Context-OS mission".

**synapse-market (vertical spinoff).** A market/TSDB vertical (`synapse-market` + its `-py`/`-ts` bindings) that only ever belonged to neither core product. It depends solely on `synapse-core` + `synapse-tsdb` and gets its own repo. The split also **cuts the `synapse-mcp -> synapse-market` edge** (the smell: the memory MCP server should never pull a domain vertical).

---

## 2. Bucket assignment — every crate → repo

Legend: **F** = shared foundation (lives in synapse-db repo, re-used by memory via submodule). LOC/status from the pre-gathered inventory.

### synapse-db (engine product)

| Crate | Role | LOC | Status |
|---|---|---|---|
| synapse-kernel **(F)** | SIMD kernels | 567 | REAL |
| synapse-core **(F)** | foundation: index/storage | 14612 | REAL (322 pub fns, 96 structs) |
| synapse-engine **(F)** | engine glue | 185 | REAL |
| synapse-ann **(F)** | ANN index | 1339 | REAL |
| synapse-fts **(F)** | full-text search | 183 | REAL |
| synapse-graph **(F)** | graph retrieval | 2269 | REAL |
| synapse-quant **(F)** | quantization | 623 | REAL |
| synapse-spann **(F)** | SPANN index | 344 | REAL |
| synapse-obs **(F)** | observability | 164 | REAL |
| synapsql | SQL-wire engine | 2701 | REAL |
| synapse-libsql | libSQL adapter | 600 | REAL |
| synapse-mysql | MySQL wire | 1075 | REAL |
| msql-srv-patched | MySQL srv patch (vendored) | — | vendored |
| synapse-mysql-async | async MySQL (if present) | — | dep-only |
| synapse-pg | Postgres wire | — | REAL |
| synapse-server | network server | 532 | REAL |
| synapse-auth | auth | 151 | REAL |
| synapse-ops | ops | 169 | REAL |
| synapse-cms | CMS | 224 | REAL |
| synapse-olap | OLAP | 92 | partial |
| synapse-mlx-olap | MLX OLAP | 316 | partial |
| synapse-tsdb | time-series store | 881 | partial |
| synapse-jit | JIT | 1285 | partial |
| synapse-iouring | io_uring (linux) | 1387 | partial |
| synapse-ring | ring buffer | 250 | partial |
| synapse-cluster | clustering | 1539 | partial |
| synapse-raft | raft | 77 | **STUB** |
| synapse-tier | tiering | 147 | partial |
| synapse-stream | streaming | 485 | partial |
| synapse-tune | autotuner | 896 | REAL (DB-side: used by server) |
| synapse-migrate | migrations | 1011 | REAL |
| synapse-license | licensing | 501 | REAL |

### synapse-memory (agent-memory product; depends on synapse-db foundation)

| Crate | Role | LOC | Status |
|---|---|---|---|
| synapse-space | memory spaces | 1161 | REAL |
| synapse-extract | extraction | 995 | partial |
| synapse-rerank | rerank | 596 | REAL |
| synapse-temporal | temporal | 152 | partial |
| synapse-learn | online learning | 1001 | REAL |
| synapsed | daemon | 2805 | REAL |
| synapse-cli | CLI | 2303 | REAL |
| synapse-mcp | MCP server | 1113 | REAL (**market dep cut**) |
| synapse-py | python binding | 392 | REAL |
| synapse-js | js binding | 110 | REAL |
| synapse-metal | Metal accel | 93 | partial |
| synapse-embed-gpu | GPU embed | 85 | partial |
| synapse-colbert | late-interaction | 1216 | recall-quality |
| synapse-splade | neural-sparse | 721 | recall-quality |
| synapse-fusion | rank fusion | 277 | recall-quality |
| synapse-multimodal | multimodal | 543 | partial |
| synapse-media | media | 1339 | partial |

> colbert/splade/fusion only depend on `kernel` (a foundation crate available via the submodule), so they *could* be shared. **Decision: keep in memory** — they are recall-quality and synapse-db has no consumer for them. Revisit only if a db surface needs neural rerank.

### synapse-market (vertical spinoff — own repo)

| Crate | Role | LOC | Status |
|---|---|---|---|
| synapse-market | market/tsdb vertical | 7389 | REAL |
| synapse-market-py | python binding | — | REAL |
| synapse-market-ts | ts binding | — | REAL |

### CUT / scaffold (archive or roadmap-only — not in any workspace)

| Crate | Disposition |
|---|---|
| synapse-wal (`synapsestore/`) | archive (legacy dup) |
| synapse-seg (`synapsestore/`) | archive (legacy dup) |
| synapse-ultra (`synapsestore/`) | archive (synapsestore dup daemon, 2626 LOC) — or fold useful bits into synapsed |
| synapse-vlog | archive (225 LOC) |
| synapse-e2e | rebuild from scratch in synapse-memory (empty, 1 LOC) |
| synapse-lib-demo / `examples/` demos | archive; ensure `examples/**/target` gitignored |
| bench/* (longmemeval, space-vs-chroma, industry) | move to a `benchmark-kit` dir under whichever product they benchmark (memory) |

---

## 3. Dependency-graph reasoning

### 3.1 Why the foundation is shared
`synapse-core` is THE foundation (14.6k LOC, 322 pub fns) and transitively depends on `ann, engine, fts, graph, kernel, obs, quant, spann`. **Both** products need core: synapse-db builds its SQL/server surfaces directly on it; synapse-memory's `space/extract/rerank/learn/...` all depend on core too. Duplicating it = drift. So the 9 foundation crates physically live in **synapse-db** and synapse-memory consumes them read-only via a pinned submodule (§4).

### 3.2 Proof: there is no `synapse-db -> synapse-memory` edge
From the authoritative cargo-metadata graph, the memory-only crates are `space, extract, rerank, temporal, learn, synapsed, cli, mcp, py, js, metal, embed-gpu, colbert, splade, fusion, multimodal, media`. Checking every db-side crate's deps:

- foundation crates depend only on each other (core → ann/engine/fts/graph/kernel/obs/quant/spann).
- `synapsql -> core, libsql, mysql, pg`; `synapse-server -> auth, graph, libsql, mysql, ops, pg, tune`; `synapse-mysql -> libsql`; `synapse-pg -> libsql`; `cms/cluster/migrate/media/ops -> core`; standalone db crates (`olap, mlx-olap, jit, iouring, ring, tsdb, raft, tier, stream, auth`) have no internal deps.
- **None reference space/extract/rerank/temporal/learn/synapsed/cli/mcp/colbert/splade/fusion/multimodal/py/js/metal/embed-gpu.**

Therefore the dependency arrow is strictly **memory → db**, never the reverse. synapse-db compiles standalone. (Enforced post-split by `cargo check` in the db repo with no submodule present, §6 gate.)

### 3.3 The `tune` fix
`synapse-tune` is consumed by `synapse-server` (`server -> ... tune`). Server is a db surface, so **tune is DB-side, not memory** — even though "tuning" sounds like a memory concern. Assigned to synapse-db.

### 3.4 The `mcp -> market` fix (the smell)
`synapse-mcp -> synapse-market` makes the memory MCP server drag in a domain vertical. **Cut this edge** during migration: remove the `synapse-market` dependency from `crates/synapse-mcp/Cargo.toml` and delete/feature-gate any `use synapse_market::*` in the mcp crate. If a market MCP surface is genuinely wanted later, it belongs in the synapse-market repo as its own `market-mcp` binary, not in the memory MCP. `cargo check -p synapse-mcp` (post-cut) is the gate.

---

## 4. Shared-dependency strategy

**Recommended (Option A): git submodule + Cargo path deps (local-first, no registry).**

synapse-db is the source of truth for the 9 foundation crates. synapse-memory vendors synapse-db at `vendor/synapse-db` (git submodule, pinned to a commit/tag) and references the foundation crates with **path** dependencies. No publishing, works fully offline, single-command bootstrap.

```bash
# in synapse-memory repo, one-time:
git submodule add git@host:org/synapse-db.git vendor/synapse-db
git submodule update --init --recursive
git -C vendor/synapse-db checkout v1.0.0   # pin
```

synapse-memory workspace `Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = ["crates/*"]

# foundation crates resolved from the pinned submodule (no registry)
[patch.crates-io]
synapse-core   = { path = "vendor/synapse-db/crates/synapse-core" }
synapse-kernel = { path = "vendor/synapse-db/crates/synapse-kernel" }
synapse-engine = { path = "vendor/synapse-db/crates/synapse-engine" }
synapse-ann    = { path = "vendor/synapse-db/crates/synapse-ann" }
synapse-fts    = { path = "vendor/synapse-db/crates/synapse-fts" }
synapse-graph  = { path = "vendor/synapse-db/crates/synapse-graph" }
synapse-quant  = { path = "vendor/synapse-db/crates/synapse-quant" }
synapse-spann  = { path = "vendor/synapse-db/crates/synapse-spann" }
synapse-obs    = { path = "vendor/synapse-db/crates/synapse-obs" }
```

A memory crate then just declares the dep normally:

```toml
# crates/synapse-space/Cargo.toml
[dependencies]
synapse-core = { version = "1.0", default-features = false }
```

CI in synapse-memory adds `submodules: recursive` to the checkout step so the path deps resolve.

**Option B (note only): private registry.** Publish the 9 foundation crates to a private cargo registry (e.g. Kellnr / cloudsmith); synapse-memory pins `synapse-core = { version = "1.0", registry = "synapse" }`. Cleaner version semantics and no submodule, but needs registry infra + a publish step on every foundation change. Adopt later if multiple downstreams consume the foundation; **start with A.**

---

## 5. Target repo layouts (Universal Project Structure v4)

### synapse-db/
```
synapse-db/
├── Cargo.toml                 # [workspace] resolver=2, members = crates/*
├── README.md  CHANGELOG.md  CONTRIBUTING.md
├── LICENSE-CORE.md  LICENSE-ENGINE.md      # dual-license, kept
├── justfile                   # setup doctor check ci pre-pr build bench
├── crates/
│   ├── synapse-kernel/  synapse-core/  synapse-engine/        # foundation
│   ├── synapse-ann/  synapse-fts/  synapse-graph/  synapse-quant/  synapse-spann/  synapse-obs/
│   ├── synapsql/  synapse-libsql/  synapse-mysql/  synapse-pg/  msql-srv-patched/
│   ├── synapse-server/  synapse-auth/  synapse-ops/  synapse-cms/  synapse-license/
│   ├── synapse-olap/  synapse-mlx-olap/  synapse-tsdb/  synapse-jit/
│   ├── synapse-iouring/  synapse-ring/  synapse-cluster/  synapse-raft/
│   └── synapse-tier/  synapse-stream/  synapse-tune/  synapse-migrate/
├── docs/
│   ├── SPEC.md                # db product spec (derived from ARCHITECTURE.md db scope)
│   ├── adr/                   # 0001-shared-foundation.md, 0002-no-memory-edge.md
│   └── archive/               # old dated bench/plan md
└── .github/workflows/
    ├── ci.yml                 # check + clippy -D warnings + nextest + build
    ├── docs.yml               # cargo doc, deny missing_docs on pub API
    ├── bench.yml  bench-nightly.yml  linux-bench.yml
    ├── security.yml           # cargo-deny + cargo-audit + osv-scanner
    └── release.yml
```

### synapse-memory/
```
synapse-memory/
├── Cargo.toml                 # members=crates/*, [patch.crates-io] -> vendor/synapse-db (§4)
├── .gitmodules                # vendor/synapse-db
├── README.md  CHANGELOG.md  CONTRIBUTING.md  LICENSE-*
├── justfile
├── vendor/synapse-db/         # git submodule (pinned)  → foundation crates
├── crates/
│   ├── synapse-space/  synapse-extract/  synapse-rerank/  synapse-temporal/
│   ├── synapse-learn/  synapsed/  synapse-cli/  synapse-mcp/        # mcp: NO market dep
│   ├── synapse-py/  synapse-js/  synapse-metal/  synapse-embed-gpu/
│   ├── synapse-colbert/  synapse-splade/  synapse-fusion/
│   ├── synapse-multimodal/  synapse-media/
│   └── synapse-e2e/           # rebuilt
├── benchmark-kit/             # ex bench/longmemeval, space-vs-chroma, industry
├── release/context-os/        # reuse clean release slice from products/release/context-os
├── docs/
│   ├── SPEC.md                # = current SPEC.md (Context-OS, 15 active crates)
│   ├── adr/                   # 0001-depends-on-db-submodule.md, 0002-mcp-market-cut.md
│   └── archive/
└── .github/workflows/
    ├── ci.yml                 # checkout submodules:recursive → check+clippy+nextest+build
    ├── docs.yml  security.yml  release.yml
```

### synapse-market/
```
synapse-market/
├── Cargo.toml                 # members=crates/*, [patch.crates-io] -> vendor/synapse-db (core,tsdb)
├── .gitmodules                # vendor/synapse-db
├── README.md  justfile
├── vendor/synapse-db/         # submodule (provides synapse-core + synapse-tsdb)
├── crates/synapse-market/  synapse-market-py/  synapse-market-ts/
├── docs/SPEC.md  docs/adr/
└── .github/workflows/
    ├── synapse-market-ci.yml  # moved from monorepo
    └── synapse-market.yml
```

---

## 6. Ordered migration recipe (execute in a git worktree)

> Run from a throwaway worktree so `main` is never touched until each repo passes its gate. **History-preserving** path = `git filter-repo` per crate-set; the simpler path = fresh repo + `git mv`. Use filter-repo (history matters for blame/bench provenance).

**Step 0 — prep.** Ensure `main` is clean (currently dirty: `Cargo.toml`, workflows, Dockerfile — commit/stash first). Tag baseline: `git tag pre-split-2026-05-29`. Confirm `examples/**/target`, `*.db`, `*.parquet` are gitignored (they are; add `examples/**/target` if missing). Install `git-filter-repo` (`brew install git-filter-repo`).

**Step 1 — make a worktree mirror to carve from.**
```bash
git worktree add ../synapse-split-wt pre-split-2026-05-29
cd ../synapse-split-wt
```

**Step 2 — carve synapse-db (primary, must compile standalone).**
```bash
git clone --no-local . /tmp/synapse-db && cd /tmp/synapse-db
git filter-repo \
  --path crates/synapse-kernel --path crates/synapse-core --path crates/synapse-engine \
  --path crates/synapse-ann --path crates/synapse-fts --path crates/synapse-graph \
  --path crates/synapse-quant --path crates/synapse-spann --path crates/synapse-obs \
  --path crates/synapsql --path crates/synapse-libsql --path crates/synapse-mysql \
  --path crates/synapse-pg --path crates/msql-srv-patched --path crates/synapse-mysql-async \
  --path crates/synapse-server --path crates/synapse-auth --path crates/synapse-ops \
  --path crates/synapse-cms --path crates/synapse-license --path crates/synapse-olap \
  --path crates/synapse-mlx-olap --path crates/synapse-tsdb --path crates/synapse-jit \
  --path crates/synapse-iouring --path crates/synapse-ring --path crates/synapse-cluster \
  --path crates/synapse-raft --path crates/synapse-tier --path crates/synapse-stream \
  --path crates/synapse-tune --path crates/synapse-migrate \
  --path LICENSE-CORE.md --path LICENSE-ENGINE.md --path CHANGELOG.md --path CONTRIBUTING.md
```
Then rewrite the root `Cargo.toml` `[workspace] members` to **only** the db crate list above (drop every memory/market/bench/synapsestore path; remove `synapsestore/crates/*`). Add `docs/SPEC.md` (db scope, from ARCHITECTURE.md), `docs/adr/0001-shared-foundation.md`, `docs/adr/0002-no-memory-edge.md`, a `justfile`, and the consolidated `.github/workflows/` (see Step 5).
**GATE:** `cargo check --workspace && cargo clippy --workspace -- -D warnings && cargo nextest run` — must be green with **no submodule present** (proves §3.2).

**Step 3 — carve synapse-market (cut the mcp edge happens in Step 4, but market is independent).**
```bash
git clone --no-local . /tmp/synapse-market && cd /tmp/synapse-market
git filter-repo \
  --path crates/synapse-market --path crates/synapse-market-py --path crates/synapse-market-ts
```
Add submodule `vendor/synapse-db`, `[patch.crates-io]` for `synapse-core` + `synapse-tsdb`, rewrite `members = ["crates/*"]`, move `synapse-market-ci.yml` + `synapse-market.yml` into `.github/workflows/`.
**GATE:** `git submodule update --init && cargo check --workspace`.

**Step 4 — carve synapse-memory (depends on db submodule; cut mcp→market).**
```bash
git clone --no-local . /tmp/synapse-memory && cd /tmp/synapse-memory
git filter-repo \
  --path crates/synapse-space --path crates/synapse-extract --path crates/synapse-rerank \
  --path crates/synapse-temporal --path crates/synapse-learn --path crates/synapsed \
  --path crates/synapse-cli --path crates/synapse-mcp --path crates/synapse-py \
  --path crates/synapse-js --path crates/synapse-metal --path crates/synapse-embed-gpu \
  --path crates/synapse-colbert --path crates/synapse-splade --path crates/synapse-fusion \
  --path crates/synapse-multimodal --path crates/synapse-media \
  --path bench/longmemeval --path bench/space-vs-chroma --path bench/industry \
  --path SPEC.md --path CHANGELOG.md --path CONTRIBUTING.md
```
Then:
1. `git submodule add <synapse-db-url> vendor/synapse-db` and pin to the synapse-db tag.
2. Add the `[patch.crates-io]` block from §4; rewrite `members = ["crates/*"]` (no foundation crates as members — they come from the submodule).
3. **Cut mcp→market:** delete the `synapse-market` line in `crates/synapse-mcp/Cargo.toml`; feature-gate/remove `use synapse_market` in the mcp source. `cargo check -p synapse-mcp` must pass.
4. Promote current `SPEC.md` to `docs/SPEC.md`; move `bench/*` → `benchmark-kit/`; reuse `products/release/context-os/` as `release/context-os/`; rebuild `crates/synapse-e2e`.
**GATE:** `git submodule update --init --recursive && cargo check --workspace && cargo clippy --workspace -- -D warnings && cargo nextest run`.

**Step 5 — CI consolidation (each repo).** Collapse the overlapping `ci.yml` / `rust-ci.yml` / `quality.yml` into one `ci.yml` per repo (checkout → fmt-check → `clippy -D warnings` → `nextest` → `build`). Keep `docs.yml` (add `RUSTDOCFLAGS="-D missing_docs"` on the public API so "all functions cleanly documented" is enforced), `security.yml` (cargo-deny + cargo-audit + osv-scanner), `release.yml`, and bench workflows in synapse-db. synapse-memory + synapse-market CI checkouts use `with: submodules: recursive`.

**Step 6 — docs cleanup (each repo).** Keep `ARCHITECTURE.md/SPEC.md/README/CHANGELOG/CONTRIBUTING`; move `BENCH_*`, `PROD_READY_*`, `PUBLISH_STATUS_*`, `CORRECTIVE-ACTION-PLAN-*`, `LAUNCH_CHECKLIST_*`, `RELEASE_NOTES_*`, `HN_LAUNCH`, `KNOWN-ISSUES`, `tuning.md`, and the ~90 dated `docs/*` into `docs/archive/`. Resolve the SPEC↔ARCHITECTURE contradiction: synapse-memory `docs/SPEC.md` = the Context-OS spec; synapse-db `docs/SPEC.md` = the engine scope distilled from `ARCHITECTURE.md`.

**Step 7 — finalize.** Push three repos. In synapse-memory/synapse-market, verify `vendor/synapse-db` points at the published synapse-db tag. Run each repo's `just ci` once and capture output. Archive `synapsestore/`, `synapse-vlog`, `synapse-ultra`, `synapse-lib-demo` to an `archive/` ref (not carried into any product). Leave the original monorepo `main` intact behind `pre-split-2026-05-29` until all three repos are green.

---

## 7. Risks + rollback

| Risk | Likelihood | Mitigation |
|---|---|---|
| Hidden `db → memory` dep not in metadata (e.g. dev-dep, feature-gated `use`) | med | Step 2 gate compiles synapse-db **with no submodule** — any leak fails `cargo check` loudly. |
| `mcp→market` cut leaves dangling `use synapse_market` | med | `cargo check -p synapse-mcp` immediately after cut; grep `synapse_market` across memory crates. |
| Submodule version skew (memory builds against wrong foundation) | med | Pin submodule to an immutable **tag**; CI fails if `vendor/synapse-db` HEAD ≠ pinned. |
| `filter-repo` drops a shared file (LICENSE, build script, vendored patch) | med | Explicitly `--path` LICENSE-CORE/ENGINE + `msql-srv-patched`; diff `cargo metadata` member count pre/post. |
| Bench/example data (3.2G in `examples/`) bloats new repos | low | Confirm `examples/**/target`, `*.db`, `*.parquet` gitignored before clone; carve only `bench/*` source dirs into memory. |
| `tune` mis-bucketed to memory by name | resolved | §3.3 — `server → tune`, stays DB-side. |
| Dirty `main` carried into split | low | Step 0 commits/stashes; carve from the `pre-split-2026-05-29` tag, not working tree. |

**Rollback:** everything is carved from clones of the tagged baseline; the source monorepo is untouched. To abort, `git worktree remove ../synapse-split-wt` and delete `/tmp/synapse-{db,memory,market}`. `main` + `pre-split-2026-05-29` are the recovery point. No new repo is made canonical until its Step-2/3/4 gate is green and `just ci` has been run once with captured output.
