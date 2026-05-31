# Synapse Context OS Plan — 2026-05-18

## Product direction

Synapse should be the local-first Context OS for AI agents: every prompt starts with the best available context, bounded to budget, cited, freshness-aware, and improved by feedback.

## Canonical repo

- Canonical path: `/Users/master/projects/synapse`
- Secondary worktrees under `/Users/master/conductor/workspaces/synapse/*` are not the product baseline.
- Current baseline: `main` at `d82d1fa`, ahead of origin by 254 commits.

## Implemented MVP

In `crates/synapse-cli`:

- `synx context <query>` — bounded context pack; hybrid → lexical → recent timeline fallback; Markdown or JSON.
- `synx remember --kind <kind> <text>` — typed memory with `kind`, `freshness`, `confidence`, `observed_at` metadata and embeddings by default.
- `synx doctor [--fix] [--json]` — quick_check, docs/vectors, duplicate hash groups, missing vectors, embedding cache, fallback readiness; safe FTS optimize with `--fix`.
- `synx fallback <query>` — robust search when semantic path is thin.
- `synx fresh-context --cwd <repo> --prompt <task>` — version-aware package/API context from local manifests.
- `synx prime <repo>` — repo startup brief: git state, source docs, likely commands, recent memories, doctor/context/freshness commands.

Verification:

```bash
rtk cargo fmt
rtk cargo check -p synapse-cli
cargo run -q -p synapse-cli -- -f /tmp/synx-worldbest-test.db remember --kind decision "..."
cargo run -q -p synapse-cli -- -f /tmp/synx-worldbest-test.db context "..."
cargo run -q -p synapse-cli -- -f /tmp/synx-worldbest-test.db doctor --json
cargo run -q -p synapse-cli -- -f /tmp/synx-worldbest-test.db prime .
release/context-os/verify.sh
SYNAPSE_VERIFY_INSTALL=1 SYNAPSE_VERIFY_BUILD_PROFILE=dev release/context-os/verify.sh
SYNAPSE_SERVICE_OS=Darwin SYNAPSE_SERVICE_DRY_RUN=1 release/context-os/service.sh install
SYNAPSE_SERVICE_OS=Linux SYNAPSE_SERVICE_DRY_RUN=1 release/context-os/service.sh install
cargo run -p longmemeval --no-default-features -- --rerank-top 0
```

## Release slice shipped

- `release/context-os/package.sh` creates a buildable source tarball with the
  minimal crate set needed for `synx`, `synapsed`, and `synapse-mcp`.
- The package excludes maintainer data: `brain.db`, WAL/SHM files,
  `.emb-cache`, `.claude`, `.codex`, `node_modules`, `file-history`, and local
  home paths.
- The optional `synapse-market` MySQL Git shim is removed from the staged
  Context OS package so first install does not depend on an external Git tag.
- `release/context-os/verify.sh` covers clean DB use, feedback, freshness,
  doctor, package install smoke, and service dry-run install.
- Source packages remain the default. Binary packages require
  `SYNAPSE_PACKAGE_INCLUDE_BIN=1`, a `SYNAPSE_RELEASE_TARGET` label, and all
  three binaries (`synx`, `synapsed`, `synapse-mcp`) in the selected bin dir.
- LongMemEval-S has a no-download quality baseline:
  `Recall@5=0.640`, `Recall@10=0.640`, `0` errors on the 50-question subset.
  See `docs/LONGMEMEVAL_RESULTS_2026-05-25.md`.

## Next build order

1. **Feature hygiene** — fix pre-existing cfg drift: `crdt`, `fts-tantivy` vs `tantivy-fts`, `mmap`, `sign`, `vec-hnsw`.
2. **Context ranking** — prefer memory types in this order: decision/fact/bugfix/benchmark/preference/research/session/note.
3. **Learning loop** — log `context` retrieval route + chosen ids; connect `feedback` to route reward.
4. **Freshness guard** — if query contains latest/current/pricing/API/model/version, combine `context` with `fresh-context` and mark stale cached facts.
5. **Project brief** — `synx prime` shipped; continue hardening source-order ranking and project-specific command detection.
6. **Doctor autofix** — safe `db-verify`, `db-repair`, FTS optimize, missing-vector report, backup age check.
7. **Workspace slimming** — separate product workspace from experimental crates to reduce build friction.

## Non-goals for core

- Do not merge conductor worktrees blindly.
- Do not put Qdrant storage, WordPress dumps, generated parser artifacts, or bench result dumps into core.
- Do not make graph/OLAP/TSDB the primary product promise.

## Core promise

> Best context, not biggest context.

Synapse is the cockpit; FTS/vector/graph/freshness tools are engines behind it.
