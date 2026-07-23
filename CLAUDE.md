# Synapse-Memory — project CLAUDE.md

Local-first **Context OS** for AI agents: bounded, cited, freshness-aware context with feedback. One SQLite-backed local brain — no Docker, no cloud. CLI + daemon + MCP tooling. Core promise: **best context, not biggest context.**

- Crate: `synapse-core` (crates.io) · MIT · Repo: https://github.com/Supersynergy/synapse-agent-memory
- Local: `~/BASE/projects/synapse-memory` · current branch: **`split-memory`** (WIP)
- Runtime socket: `/tmp/synapse.sock` :9477 (SQLite fallback `~/.synapse/brain.db`)

## Stack
Rust workspace (edition 2024). SimSIMD kernels (1-bit 71× / int8 46× / MRL-128 35× / f16 4×). MRL/f16/Hamming vectors. Members in `crates/*` (py, js, metal, embed-gpu, colbert, splade) + vendored `vendor/synapse-db`.

## Commands (just)
```bash
just check        # default gate (fmt + clippy + check)
just test         # cargo nextest
just ci           # check + test          ← full gate
just build
just check-layers # architecture/layer boundary check
```
Direct: `cargo clippy --all-targets --all-features -- -D warnings` · `cargo nextest run`.

## Quality / security
`deny.toml` (cargo-deny) · `audit.toml` (cargo-audit) · `clippy.toml`. Run before release. `ARCHITECTURE.md` = layer map; `CONTRIBUTING.md` = workflow.

## Lean-code notes
Vendored code (`vendor/synapse-db`) MUST carry provenance (upstream URL + commit SHA + sync-date) and stay visible to `cargo audit` — invisible copies are un-patchable CVEs (xz/CVE-2024-3094 era). Correctness-critical surface (vectors, SQLite, parsers) → KEEP maintained deps, never self-roll. Apply `/leancode` before adding crates.

## Release flow
`CHANGELOG.md` newest-first (currently `Unreleased`). Merge `split-memory` → cut a tagged version before claiming release.

Inherits global rules `~/.claude/CLAUDE.md` + workspace `~/BASE/projects/CLAUDE.md`.
