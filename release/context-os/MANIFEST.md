# Context OS Release Manifest

## Included

- `README.md` — clean-user onboarding for Mac and Linux.
- `install.sh` — source install that builds and installs `synx`, `synapsed`, and `synapse-mcp`.
- `service.sh` — user-level macOS launchd / Linux systemd service installer.
- `verify.sh` — offline smoke for init, remember, context, feedback, prime, fresh-context, doctor, and db-verify.
- `package.sh` — release tarball builder with private-data guards.
- `CHECKLIST.md` — release gates for CLI, packaging, service setup, and product behavior.
- `RELEASE_NOTES.md` — user-facing changes and explicit non-claims.
- `VERIFICATION.md` — current gate evidence and remaining completion gaps.
- `sample/seed.jsonl` — tiny public sample memories for demos.
- `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, and the minimal crate set
  needed to build `synx`, `synapsed`, and `synapse-mcp` from source.
- `python/migrations/add_rerank_log.sql` — build-time migration included by
  `synapse-rerank`.
- `integrations/codex/` — reversible Codex hook installer, crash-safe
  checkpoint journal, recovery injection, and focused tests.
- Optional `bin/synx`, `bin/synapsed`, and `bin/synapse-mcp` only when building
  a target-labelled binary package with `SYNAPSE_PACKAGE_INCLUDE_BIN=1`.

## Explicitly Not Included

- `~/.synapse/brain.db`
- `.synapse/.emb-cache`
- Claude/Codex session logs
- Synapse checkpoint journals from `~/.synapse/checkpoints/`
- file-history snapshots
- node_modules or generated benchmark dumps
- private memories, decisions, or known-facts from the maintainer machine
- optional market MySQL Git shim in the Context OS source package

## User-Owned Paths

- Database: `$SYNAPSE_DB` or `$HOME/.synapse/brain.db`
- Binaries: `$SYNAPSE_PREFIX/bin` or `$HOME/.local/bin`
- Cache: adjacent `.emb-cache` next to the selected database

## Current Scope

This release folder packages the Context OS workflow. It deliberately does not
make graph, OLAP, TSDB, WordPress, Surreal parity, or secure customer licensing
part of the default first-run path.
