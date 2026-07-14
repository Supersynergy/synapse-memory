# Changelog

Notable changes to the public Synapse Memory product are recorded here. Earlier
engine-lab wave notes remain available in Git history; they are not part of the
portable release contract.

## [Unreleased]

### Changed

- Standardized the public product name as **Synapse Memory** across the CLI,
  README, release workflow, installers, release notes, and social preview.
- Rewrote the product explanation around human continuity, privacy, provenance,
  and the smallest useful context.
- Renamed the canonical release tag family to `synapse-memory-v*`.

### Removed

- Removed legacy product naming, broad engine-lab marketing, unshipped feature
  claims, competitor positioning, cached Python bytecode, and stale local
  benchmark artifacts from the public release surface.

## [1.0.1-rc.1] - 2026-07-14

### Added

- One portable native `synx` binary for macOS, Linux-musl, and Windows on x86-64
  and ARM64.
- Local SQLite memory, typed capture, lexical retrieval, bounded cited context,
  project grounding, feedback calibration, health checks, backup/restore,
  integrity signatures, graph basics, and CRDT federation.
- Optional, reversible Codex checkpoint integration that excludes transcript and
  tool-output contents.
- Fail-closed SHA-256 installers, rollback, data-preserving uninstall, build
  metadata, and exact first- and third-party license payloads.

### Security

- Portable dependency closure passes RustSec, `cargo-deny`, license closure,
  native-artifact, archive-content, and corrupted-checksum gates.
- All GitHub Actions in the canonical memory workflows are commit-SHA pinned.

### Deliberate boundary

- The portable channel excludes the proprietary engine, daemon/MCP transport,
  model runtime, PDF parser, encrypted packs, sharding, Tantivy, and unrelated
  database, market, multimodal, and benchmark experiments.
- Retrieval is lexical in this channel. Unsupported semantic operations fail with
  a clear feature message.

[Unreleased]: https://github.com/Supersynergy/synapse/compare/synapse-memory-v1.0.1-rc.1...HEAD
[1.0.1-rc.1]: https://github.com/Supersynergy/synapse/releases/tag/synapse-memory-v1.0.1-rc.1
