# Synapse Agent Memory release gates

No public tag until every P0 gate has a machine-readable log or artifact.

## P0 — blocks publication

- [x] Workspace, CLI, `VERSION`, installer constants, release notes, and canonical
      `synapse-agent-memory-v*` tag family agree.
- [x] Owner explicitly approves the FSL/MIT boundary through repository variable
      `SYNAPSE_RELEASE_LICENSE_APPROVED=true`.
- [x] The release tree contains no memory database, key, checkpoint, transcript,
      cache, model, generated graph, or local absolute path.
- [x] Rustfmt, portable tests, Clippy, RustSec, dependency policy, license closure,
      and release-diff secret scan pass.
- [x] Native runners build and execute macOS, Linux, and Windows on x86-64 and ARM64.
- [x] Every archive contains one native `synx`, build metadata, README, exact
      licenses, and a SHA-256 sidecar. Script wrappers are rejected.
- [x] Init, temporal/priority remember, supersession filtering, pass/fail feedback,
      freshness, verified backup-before-repair, backup/restore, doctor, install,
      corrupt-checksum rejection, rollback, and data-safe uninstall pass.
- [x] Windows TCP federation compiles; Unix-only transports fail with a useful
      feature message on unsupported targets.
- [x] Codex forced-disconnect recovery passes without transcript storage or blind
      mutation replay.

## P1 — blocks stronger claims

- [ ] Reproducible task-level comparison against current memory alternatives.
- [ ] Citation fidelity and context-budget adherence measured on real coding tasks.
- [ ] Install time, first query, p50/p95, peak RSS, and disk growth across all targets.
- [ ] Semantic channel with pinned model, offline cache, license review, and lexical fallback.
- [x] Deterministic English/German/quarter parsing, event-range filtering, and
      supersession have executable regression tests.
- [ ] Temporal and citation quality measured on real multi-session coding tasks.

Until those gates pass, public copy stays with the verified promise: small,
deterministic, local coding-agent continuity.

## P2 — later distribution work

- Homebrew and WinGet/Scoop manifests consuming the same checksummed archives.
- Signed macOS artifacts and Windows Authenticode when distribution volume
  justifies certificate and maintenance cost.
- Any MCP, daemon, semantic, or hosted channel gets an independent platform,
  dependency, security, and quality gate.

Current evidence: [PROOF.md](PROOF.md).
