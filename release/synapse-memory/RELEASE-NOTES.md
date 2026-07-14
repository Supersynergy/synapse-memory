# Synapse Memory 1.0.1-rc.1

Your agents forget. Your work shouldn't.

This first portable release candidate keeps the decisions, cited context, and
recovery breadcrumbs behind coding work in one private SQLite file.

## What ships

- One native Rust `synx` binary for macOS, Linux-musl, and Windows on x86-64 and
  ARM64.
- Typed local memory, exact lexical retrieval, bounded cited context, project
  grounding, local-manifest freshness, feedback calibration, health checks,
  backup/restore, signatures, graph basics, and CRDT federation.
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

## Deliberate boundary

This download excludes the proprietary engine, daemon/MCP transport, model
runtime, PDF parser, encrypted packs, sharding, Tantivy, and unrelated database,
market, multimodal, and benchmark experiments.

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
