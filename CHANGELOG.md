# Changelog

Notable changes to the public Synapse Memory product are recorded here. Earlier
engine-lab wave notes remain available in Git history; they are not part of the
portable release contract.

## [Unreleased]

## [1.1.0-rc.1] - 2026-07-14

### Added

- Added dual timestamps: capture time remains automatic; `remember --occurred-at`
  records when an event happened. Context understands ISO dates, English/German
  relative dates, and `Q1`–`Q4` ranges.
- Added bounded `critical|high|normal|low` memory priority and explicit
  `--supersedes <docs.id>` history links.
- Added context-time exclusion of superseded truth, known Telepathy/status noise,
  stale/archived records, plus machine-readable filter diagnostics.
- Added explicit context `--gate pass|fail` feedback with actually used ids,
  reward-poisoning checks, score buckets, memory-type rewards, and automatic local
  calibration.
- Added safe FTS self-healing: canonical SQLite must pass first; a private
  brainpack is created, restored, and checked before the derived index changes.
  Repair state is written to `health_events` and interrupted repairs remain visible.

### Changed

- Standardized the public product name as **Synapse Memory** across the CLI,
  README, release workflow, installers, release notes, and social preview.
- Rewrote the product explanation around human continuity, privacy, provenance,
  and the smallest useful context.
- Renamed the canonical release tag family to `synapse-memory-v*`.
- Made release-asset publication fail when file globs are empty, moved checkout
  before artifact download, and upgraded artifact actions to their Node 24 lines.
- Context budgets now apply to the emitted blocks, compact Unicode safely, and log
  the precise candidates and normalized scores used by the learning loop.
- `db-repair` now states and enforces its real boundary: FTS only; vectors and
  canonical documents are never silently rebuilt or deleted.

### Security

- Feedback rejects document ids that were not part of the referenced context pack.
- Self-healing refuses mutation after failed canonical or backup integrity checks.
- Historical Telepathy transport stays outside the portable release; its known
  status/notification records are filtered from context without data deletion.

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

[Unreleased]: https://github.com/Supersynergy/synapse/compare/synapse-memory-v1.1.0-rc.1...HEAD
[1.1.0-rc.1]: https://github.com/Supersynergy/synapse/compare/synapse-memory-v1.0.1-rc.1...synapse-memory-v1.1.0-rc.1
[1.0.1-rc.1]: https://github.com/Supersynergy/synapse/releases/tag/synapse-memory-v1.0.1-rc.1
