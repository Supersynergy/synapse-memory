# Synapse Memory 1.0.1-rc.1

First portable release candidate for local coding-agent memory.

## What ships

- One native Rust `synx` binary per macOS, Linux-musl, and Windows target on
  x86-64 and ARM64.
- Local SQLite memory, typed capture, lexical retrieval, bounded cited context,
  feedback, integrity checks, backup/restore, signatures, graph basics, and
  offline local-manifest freshness.
- Checksummed archives with build metadata, exact first-party terms, and the
  locked portable dependency-license report.
- Optional Codex checkpoint adapter. Journals are append-only and fsynced;
  snapshots are atomic; transcript and tool output are not persisted.

## Deliberate exclusions

- No proprietary engine, daemon, MCP server, market/database experiments, ONNX
  runtime, model download, network client, PDF parser, `age` encryption, IVF
  sharding, Rayon, or Tantivy in this package.
- Retrieval is lexical in the portable channel. Explicit semantic operations
  fail with a clear feature message.

## Install

After the tagged GitHub Actions matrix has published all six checksummed assets:

```sh
curl -fsSL https://raw.githubusercontent.com/Supersynergy/synapse/main/release/synapse-memory/install.sh | sh
```

The installer selects the native target, pins this release version, verifies its
SHA-256 sidecar, rejects unexpected archive entries, keeps `synx.previous`, and
does not remove `~/.synapse/brain.db`.

## Upgrade and rollback

The installer preserves an existing binary as `synx.previous` and never deletes
`~/.synapse/brain.db`. The uninstaller removes the installed binary and leaves
memory intact unless the user explicitly removes that file.

## Publication gate

The release workflow requires owner approval of the FSL/MIT boundary through
repository variable `SYNAPSE_RELEASE_LICENSE_APPROVED=true`; it then publishes
only after all six native GitHub Actions runners build, smoke-test, package, and
verify their assets.
