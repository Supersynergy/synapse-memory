# Vendored: synapse-db

This directory vendors the `synapse-db` workspace. It is tracked directly in this
repository (absorbed from a git submodule on 2026-10-01) so that a plain
`git clone` + `cargo build` works everywhere, including CI and Windows.

## Provenance

- Upstream: local project `/Users/master/projects/synapse-db` (branch `split-db`)
- Synced commit: `6a7cc5de15a2308fb05a8e0b68b5218f77566a89`
  ("sec: 0600 keygen secret + brain.db chmod on Store::open")
- Sync date: 2026-10-01

## Updating

Sync upstream changes by copying the crate sources over this tree, then updating
the commit SHA + date above. Do not drop `synapse-db` patches that exist only
here without checking `git log -- vendor/synapse-db` first.
