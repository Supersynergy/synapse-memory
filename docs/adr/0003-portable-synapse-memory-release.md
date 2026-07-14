# ADR 0003: Binary-first portable Synapse Memory release

- Status: proposed, implemented as a release candidate
- Date: 2026-07-13

## Context

The repository contains a useful memory CLI plus daemon, MCP, database proxies,
market tooling, multimodal work, benchmarks, and experimental crates. Existing
release paths mix source builds, MCP-only artifacts, stale binary names, Unix-only
transports, and multiple tag/version narratives. The maintainer machine may also
place a private Python routing wrapper at `~/.local/bin/synx`, which must never be
mistaken for the public Rust product.

The former portable CLI command did not compile without embedding features because
`synapse-cli` imported the ONNX embedder unconditionally. The daemon and MCP server
use Unix sockets and therefore cannot define a universal Windows contract today.

## Decision

The end-user product is `synx`: one native Rust binary and one user-owned SQLite
database. `release/synapse-memory/` is the canonical binary package.

The default portable profile uses:

```text
cargo build -p synapse-cli --no-default-features
```

It ships lexical/timeline context, typed memory, dual timestamps, bounded priority,
explicit supersession, pass/fail feedback, freshness, graph basics,
backup/restore/merge, signing, import/export, and TCP federation. Writes succeed
without embeddings and say so. Explicit vector operations fail clearly. Doctor does
not report absent embeddings as a health defect in this profile.

The v1.1 truth path filters known transport/status noise and superseded memories
before packing context. Self-healing is restricted to rebuildable FTS state and may
run only after canonical SQLite and a restored pre-repair brainpack both pass
integrity checks. Canonical documents and vectors are never silently rewritten.

Semantic embeddings, daemon, MCP, and Codex hooks are optional packages/adapters.
They cannot block installation of the portable memory core.

Release archives are prebuilt on native macOS, Linux, and Windows runners for x64
and ARM64. Installers require SHA-256 sidecars and preserve existing databases.
Packaging rejects shebang/script wrappers.

## Consequences

Positive:

- Install has no compiler, Python, Node, Docker, database service, cloud account, or
  provider key.
- Platform failures are isolated from experimental workspace crates.
- The product claim becomes narrow and testable: local coding-agent continuity.
- The private maintainer wrapper and public Rust binary cannot be confused silently.

Costs:

- Portable recall is lexical until semantic assets pass separate distribution and
  eval gates.
- MCP and warm daemon behavior are not part of the Windows promise.
- Existing release scripts remain legacy until redirects and old workflow cleanup
  are approved.
- “Better than established memory tools” requires equal-workload memory evals, not
  the current engine microbenchmarks.
