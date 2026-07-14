# Synapse Memory

Portable, local-first memory for coding agents. One native Rust binary, one
SQLite-backed brain, no Docker, no cloud account, and no API key for the default
product path.

> **Release status: CANDIDATE.** Local package, install, recovery, and portable
> RustSec closure pass. Public publishing still requires clean commits, owner
> license approval, and the six native CI jobs.

## Two-minute path

macOS or Linux, from a checksummed `ctxos-v*` release:

```sh
curl -fsSL https://raw.githubusercontent.com/Supersynergy/synapse/main/release/synapse-memory/install.sh | sh
synx remember --kind decision "Use the release verifier before publishing."
synx context "What must pass before release?" --mode coding
```

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/Supersynergy/synapse/main/release/synapse-memory/install.ps1 | iex
synx remember --kind decision "Use the release verifier before publishing."
synx context "What must pass before release?" --mode coding
```

The installer is version-pinned and fails closed unless the matching archive and
SHA-256 sidecar exist on GitHub Releases.

## What this package is

| Included | Contract |
|---|---|
| `synx` | Native Rust CLI; scripts and local wrappers are rejected by packaging |
| `brain.db` | User-created SQLite memory file under `~/.synapse/`; never bundled |
| Typed memory | Decisions, facts, bugs, benchmarks, research, ADRs, and notes |
| Cited context | Bounded packs with stable ids, retrieval route, and feedback hint |
| Freshness | Local manifest/lockfile context without registry access |
| Durability | Backup, restore, merge, integrity check, signatures, and CRDT federation |
| Recovery adapter | Optional Codex checkpoint integration; separate from the Rust core |

First-party license boundary: `synapse-core` is FSL-1.1-ALv2; CLI/graph/learning
utility crates are MIT. Both exact texts ship in `LICENSES/`. The proprietary
`synapse-engine` is not in the portable dependency graph or archive.
The exact portable multi-target dependency inventory and license texts ship as
`THIRD-PARTY-LICENSES.html`; `licenses.sh check` fails when it drifts.

The portable binary deliberately excludes the ONNX embedding runtime. It always
supports lexical retrieval, timeline fallback, cited context, graph commands,
freshness, feedback, backup, and merge. Explicit vector operations fail with a
clear message instead of silently pretending to be semantic search.

Also excluded from this minimal channel: proprietary engine, Rayon/turbo/Tantivy,
IVF sharding, encrypted `age` packs, and PDF parsing. Their commands are absent or
return a precise feature message; normal memory, RSS/web/text ingest, and plain
backup/restore remain available.

Semantic binaries are a later release channel. They must pass the same platform
matrix and memory-quality evals before becoming the default.

## Install from a local release asset

```sh
SYNAPSE_RELEASE_BASE="file:///absolute/path/to/dist" \
  release/synapse-memory/install.sh
```

Useful overrides:

```sh
SYNAPSE_PREFIX="$HOME/.local" \
SYNAPSE_DB="$HOME/.synapse/brain.db" \
SYNAPSE_REPO="https://github.com/Supersynergy/synapse" \
  release/synapse-memory/install.sh --version 1.0.1-rc.1
```

The installer pins the version in `VERSION` instead of following GitHub's global
`latest` release, which may belong to the frozen legacy `v*` channel. It requires
a SHA-256 sidecar; missing or mismatched checksums are a hard failure. It then
initializes the database and runs `doctor`.

## Build the portable Rust binary

```sh
TARGET="$(rustc -vV | sed -n 's/^host: //p')"
cargo build --locked --profile release-hardened \
  --target "$TARGET" \
  -p synapse-cli \
  --no-default-features
```

Build a host release asset from an existing binary:

```sh
TARGET="$(rustc -vV | sed -n 's/^host: //p')"
SYNAPSE_BIN="target/$TARGET/release-hardened/synx" \
SYNAPSE_TARGET="$TARGET" \
  release/synapse-memory/package.sh
```

Local end-to-end verification:

```sh
TARGET="$(rustc -vV | sed -n 's/^host: //p')"
SYNX_BIN="target/$TARGET/release-hardened/synx" \
  release/synapse-memory/verify.sh
```

Prove the current release overlay from a clean detached `HEAD` checkout without
committing or touching the active worktree:

```sh
release/synapse-memory/fresh-snapshot.sh
```

Local macOS ARM64 footprint evidence:
[evidence/macos-aarch64-local.md](evidence/macos-aarch64-local.md). Regenerate with
`benchmark.py`; process startup is included and no daemon/model/network is used.

## Release target matrix

| OS | Architecture | Asset |
|---|---|---|
| macOS | Apple Silicon | `synapse-memory-aarch64-apple-darwin.tar.gz` |
| macOS | Intel | `synapse-memory-x86_64-apple-darwin.tar.gz` |
| Linux | x86-64, static musl | `synapse-memory-x86_64-unknown-linux-musl.tar.gz` |
| Linux | ARM64, static musl | `synapse-memory-aarch64-unknown-linux-musl.tar.gz` |
| Windows | x86-64 | `synapse-memory-x86_64-pc-windows-msvc.zip` |
| Windows | ARM64 | `synapse-memory-aarch64-pc-windows-msvc.zip` |

Tier-1 means the asset is built and its add/context/feedback/backup flow runs on a
native CI runner. A target is not advertised merely because Rust can compile it.

## Read next

- [FEATURES.md](FEATURES.md) — capability map and product boundary
- [COMPETITOR-GAP.md](COMPETITOR-GAP.md) — honest wedge against established memory tools
- [RELEASE-GATES.md](RELEASE-GATES.md) — required proof before public release
- [PROOF.md](PROOF.md) — exact local results and remaining external STOPs
- [MANIFEST.md](MANIFEST.md) — files and archive contract
- [RELEASE-NOTES.md](RELEASE-NOTES.md) — exact candidate scope and upgrade notes
- [evidence/macos-aarch64-local.md](evidence/macos-aarch64-local.md) — measured local footprint
