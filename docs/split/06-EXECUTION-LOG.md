# Synapse Split — Execution Log

**Date:** 2026-05-31 · **Mode:** physical split, local-only (no push).

## Result — three repos under `~/projects/`, all build green

| Repo | Crates | History | `cargo check --workspace` |
|---|---|---|---|
| `synapse-db` (branch `split-db`) | 30 | 185 commits (filter-repo preserved) | **exit 0 — standalone, no submodule** |
| `synapse-memory` (branch `split-memory`) | 17 | 149 commits | **exit 0 — via `vendor/synapse-db` submodule** |
| `synapse-market` (branch `split-market`) | 3 | 73 commits | **exit 0 — via `vendor/synapse-db` submodule** |

The "no `synapse-db → memory` edge" thesis is **proven**: synapse-db compiles with no submodule present.

Each repo has: `.github/workflows/ci.yml` (fmt · clippy · check · nextest), `.github/workflows/security.yml` (cargo-deny + cargo-audit), `justfile` (setup/check/test/ci/build).

## How it was done
- Baseline tag `pre-split-2026-05-29` = **working-tree snapshot** (via `git add -A` into a temp `GIT_INDEX_FILE` → `commit-tree`; the original repo's branch/index/working-tree were never mutated).
- `git filter-repo --paths-from-file` carved each crate-set (history preserved per crate).
- Foundation crates (`kernel/core/engine/ann/fts/graph/quant/spann/obs`) live in `synapse-db`; memory & market vendor them via a `vendor/synapse-db` git submodule and **path-retargeted** their foundation deps to it.
- `mcp → market` edge cut: removed dep + `smx_*` tool code from `synapse-mcp/src/main.rs`.

## Deviations from `01-SPLIT-PLAN.md` (and why)
1. **Carved from working-tree snapshot, not committed HEAD.** HEAD was stale — 359 modified `.rs` + untracked files (incl. `synapse-core/src/fresh.rs`, the fixed `ann.rs`/`db.rs`). Carving HEAD produced uncompilable code (`Ann::ensure_capacity_for_tail` missing). Snapshot fixed it.
2. **Path-dep retarget, not `[patch.crates-io]`.** Internal deps are path-based, so the plan's `[patch]` block doesn't apply; rewrote `path = "../X"` → `path = "../../vendor/synapse-db/crates/X"`.
3. **Dropped stray `crates/msql-srv-patched`** — only `src/packet.rs` tracked, no `Cargo.toml`, unreferenced.
4. **Copied `python/migrations/add_rerank_log.sql`** into synapse-memory — `synapse-rerank` `include_str!`s it.
5. `synapse-py`, `synapse-metal`, `synapse-embed-gpu` set as workspace `exclude` (they were non-members in source: cdylib/experimental/feature-conflict). Build opt-in.

## Caveats / not-yet-done (honest)
- **Only `cargo check` is verified green.** `cargo fmt --check`, `clippy`, and `cargo nextest` are NOT yet run — CI will run them; expect a cleanup pass (synapse-db has ~47 dead-code warnings; clippy is not `-D warnings` yet).
- **Submodule URL is a local path** (`~/projects/synapse-db`). Before pushing, repoint `.gitmodules` to the real synapse-db git remote, then `git submodule sync`. Submodule is pinned to the carve commit (`407f466`), one commit behind db HEAD (`daf1cde`); foundation source is identical (the newer commit only added CI/justfile).
- **Not pushed** (local-only per request). Repos are on `split-*` branches.
- `synapse-mcp` lost the `smx_*` market tools (intended); rebuild them as a `market-mcp` binary in synapse-market if wanted. Stale doc-comment at `main.rs:3` still mentions market.
- Original monorepo (`~/projects/synapse`, branch `main`) is **untouched** — working tree (505 dirty files) preserved; rollback = delete the three new dirs.

## Next steps
1. `just ci` in each repo → fix fmt/clippy/test failures (cleanup pass).
2. Repoint submodule URLs to real remotes; push.
3. Per `05-REPO-HYGIENE.md`: archive dated root `.md`, fix `.gitignore`, move `bench/*` → `benchmark-kit/`, reuse `release/context-os/` in synapse-memory.
4. Per-repo `docs/SPEC.md` + `docs/adr/` (synapse-db = engine scope from ARCHITECTURE.md; synapse-memory = current SPEC.md Context-OS).
