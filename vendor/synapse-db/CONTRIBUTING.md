# Contributing to Synapse

Thanks for wanting to help. Synapse is MIT for the code and CC0 for the format spec — contributions land under the same terms.

## Setup

```bash
git clone https://github.com/Supersynergy/synapse
cd synapse
# rust-toolchain.toml pins 1.95.0 automatically
cargo build --workspace
cargo test --workspace
```

For the v0.3 engine features:

```bash
cargo test -p synapse-core --features full
```

## Local check before sending a PR

Run `just check-all` before opening a PR — this runs the full fmt/clippy/test/audit/deny suite locally. Before tagging a release, run `scripts/pre_release.sh` which additionally checks semver compatibility, bloat, and the e2e smoke bench. Install the pre-push git hook via `bash scripts/setup_hooks.sh` to catch issues before they reach CI.

```bash
just check-all          # fmt + clippy + nextest + audit + deny
scripts/pre_release.sh  # before tagging only
```

## Areas that want help

- **Phase 3.2 — rkyv manifest**: replace JSON archival with zero-copy rkyv.
- **Phase 3.3 — persisted FTS / HNSW chunks**: right now both indexes rebuild on open; the design lets them live inside `.synx` chunks.
- **Conformance suite**: byte-exact test vectors at `spec/conformance/` so any language can verify its reader.
- **SDKs**: Go, Zig, Elixir, Swift — the format is CC0, the reader is <200 LOC in Python, port it.
- **Bench coverage**: add more incumbents to `bench/top20_formats.py` — every binding counts.

## Format-spec changes

`.synx` is spec-versioned. Any breaking layout change must:

1. Bump the header version byte.
2. Land with a migration path from the prior version.
3. Ship conformance vectors covering the new chunk kind.
4. Open an RFC issue tagged `rfc-synx`.

See [`docs/RFC-CALL.md`](docs/RFC-CALL.md) for the review loop.

## Code style

- Rust: `rustfmt.toml` = defaults. `clippy.toml` = strict.
- Errors: prefer `synapse_core::Error` variants over `anyhow` inside `synapse-core`; `anyhow` is fine in binaries.
- No new dependencies without justification in the PR body.
- Feature-gate anything that pulls a heavy dep (Tantivy, Automerge, Ed25519).

## Release process

1. Land PRs into `main`.
2. Bump `workspace.package.version` in `Cargo.toml`.
3. Add a `## vX.Y.Z` entry to `CHANGELOG.md`.
4. Tag `vX.Y.Z` — the GitHub release is generated from the changelog entry.

## Code of Conduct

We follow the [Rust Code of Conduct](https://www.rust-lang.org/policies/code-of-conduct). Be decent.
