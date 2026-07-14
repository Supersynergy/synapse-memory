# Synapse Memory 1.1.0-rc.1

Your agents forget. Your work — and the care behind it — shouldn't.

This release makes long-term memory more truthful: it knows when an event
happened, what answer replaced an older one, which memories mattered to the
human, and whether a context pack actually helped.

## What is new

- `remember --occurred-at` separates event time from capture time. Context date
  cues support ISO dates, English/German relative dates, and quarters.
- `--priority critical|high|normal|low` adds a bounded tie-breaker;
  `--supersedes <docs.id>` preserves old history while excluding stale truth.
- Context applies a hard character budget and reports how many Telepathy/status,
  superseded, and out-of-range records it filtered.
- Feedback now accepts explicit `--gate pass|fail` and `--used` ids, rejects ids
  outside the emitted pack, and recalibrates score buckets locally.
- `doctor --fix` checks canonical SQLite, creates a private brainpack, restores
  and checks it, then repairs only the rebuildable FTS index. Every attempt is
  auditable; interrupted repair stays visible.

## What ships

- One native Rust `synx` binary for macOS, Linux-musl, and Windows on x86-64 and
  ARM64.
- Typed local memory, exact lexical retrieval, bounded cited context, project
  grounding, local-manifest freshness, temporal truth, feedback calibration,
  safe self-healing, backup/restore, signatures, graph basics, and CRDT federation.
- Checksummed archives with build metadata, exact first-party license texts, and
  a generated report for the locked third-party dependency closure.
- Optional, reversible Codex checkpoints that exclude transcript and tool-output
  contents.

## Safety by default

- Install fails when a checksum is missing or wrong.
- Archives with unexpected or unsafe entries are rejected.
- Upgrade preserves the previous binary for rollback.
- Uninstall preserves `~/.synapse/brain.db`.
- The default path needs no cloud account, API key, model, daemon, or container.
- Repair never silently edits or deletes canonical documents or vectors.

## Deliberate boundary

This download excludes the proprietary engine, daemon/MCP transport, model
runtime, PDF parser, encrypted packs, sharding, Tantivy, and unrelated database,
market, multimodal, and benchmark experiments.

The historical Telepathy transcript tailer also remains outside this download.
Known Telepathy/status noise is filtered from context without deleting evidence;
the optional Codex adapter carries only minimal interruption checkpoints.

Retrieval is lexical in the portable channel. Unsupported semantic operations
fail with a clear message instead of pretending they worked.

## Install

macOS or Linux:

```sh
curl -fsSL https://raw.githubusercontent.com/Supersynergy/synapse/main/release/synapse-memory/install.sh | sh
```

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/Supersynergy/synapse/main/release/synapse-memory/install.ps1 | iex
```

The release workflow publishes only after all six native runners build, execute,
package, and verify their target artifact.
