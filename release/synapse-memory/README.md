# Synapse Memory portable release

This is the small, dependable download for people who want their coding agents to
remember the work that matters.

One native Rust binary. One private SQLite brain. No Docker, cloud account, API
key, model download, or background service.

## Two-minute path

macOS or Linux, from a checksummed `synapse-memory-v*` release:

```sh
curl -fsSL https://raw.githubusercontent.com/Supersynergy/synapse/main/release/synapse-memory/install.sh | sh

BRAIN="$HOME/.synapse/brain.db"
synx -f "$BRAIN" remember --kind decision --priority high \
  --occurred-at 2026-07-14 "Keep the reason, not only the result."
synx -f "$BRAIN" context "What should the next session know?" --mode coding
```

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/Supersynergy/synapse/main/release/synapse-memory/install.ps1 | iex

$Brain = "$HOME\.synapse\brain.db"
synx -f $Brain remember --kind decision --priority high `
  --occurred-at 2026-07-14 "Keep the reason, not only the result."
synx -f $Brain context "What should the next session know?" --mode coding
```

The installer is version-pinned and stops unless the matching archive and SHA-256
sidecar both exist and verify.

## What people get

| Included | Why it matters |
|---|---|
| `synx` native CLI | Starts without a runtime, daemon, container, or cloud dependency |
| Private `brain.db` | Your memory remains a file you own, inspect, back up, and move |
| Temporal typed memory | Decisions, facts, fixes, research, and ADRs keep type, confidence, priority, capture time, and optional event time |
| Supersession | New truth can replace an old memory without deleting its history |
| Bounded cited context | An agent receives useful evidence, dates, filter counts, and stable ids without replaying everything |
| Project grounding | `prime` reconnects a new session to the repository in front of it |
| Local freshness | Manifests and lockfiles ground version-sensitive work without registry access |
| Feedback loop | Explicit pass/fail and actually used ids drive local score and memory-type calibration |
| Safe self-healing | Doctor verifies canonical SQLite and a restored pre-repair pack before repairing only the derived FTS index |
| Recovery tools | Verification, backup, restore, merge, BLAKE3, and Ed25519 protect continuity |
| Optional Codex adapter | Interrupted work can resume from minimal state without storing the conversation |

The portable binary deliberately has no embedding runtime. Lexical retrieval,
timeline fallback, cited context, freshness, feedback, backup, and merge always
work. Explicit vector operations fail clearly instead of silently pretending to
be semantic search.

The old Telepathy transcript tailer is intentionally absent. It mixed useful live
continuity with status and notification noise. The portable path keeps minimal
Codex checkpoints and explicit memory promotion; context filters known Telepathy,
stale, archived, and notification records without deleting them.

## Install a specific version

```sh
SYNAPSE_PREFIX="$HOME/.local" \
SYNAPSE_DB="$HOME/.synapse/brain.db" \
SYNAPSE_REPO="https://github.com/Supersynergy/synapse" \
  release/synapse-memory/install.sh --version 1.1.0-rc.1
```

Install from a local release directory:

```sh
SYNAPSE_RELEASE_BASE="file:///absolute/path/to/dist" \
  release/synapse-memory/install.sh
```

The installer uses the exact version in `VERSION`; it never follows GitHub's
ambiguous repository-wide `latest` endpoint. Upgrade preserves the previous
binary as `synx.previous`. Uninstall preserves `~/.synapse/brain.db`.

## Build and verify

```sh
TARGET="$(rustc -vV | sed -n 's/^host: //p')"
cargo build --locked --profile release-hardened \
  --target "$TARGET" -p synapse-cli --no-default-features

SYNX_BIN="target/$TARGET/release-hardened/synx" \
  release/synapse-memory/verify.sh
```

Build a host release asset from an existing binary:

```sh
SYNAPSE_BIN="target/$TARGET/release-hardened/synx" \
SYNAPSE_TARGET="$TARGET" \
  release/synapse-memory/package.sh
```

Prove the release overlay from a clean detached checkout:

```sh
release/synapse-memory/fresh-snapshot.sh
```

## Native target matrix

| OS | Architecture | Asset |
|---|---|---|
| macOS | Apple Silicon | `synapse-memory-aarch64-apple-darwin.tar.gz` |
| macOS | Intel | `synapse-memory-x86_64-apple-darwin.tar.gz` |
| Linux | x86-64 static musl | `synapse-memory-x86_64-unknown-linux-musl.tar.gz` |
| Linux | ARM64 static musl | `synapse-memory-aarch64-unknown-linux-musl.tar.gz` |
| Windows | x86-64 | `synapse-memory-x86_64-pc-windows-msvc.zip` |
| Windows | ARM64 | `synapse-memory-aarch64-pc-windows-msvc.zip` |

A target is advertised only after a native GitHub runner builds the binary and
executes its memory/context smoke flow.

## What is intentionally not here

The portable archive excludes the proprietary engine, daemon/MCP transport,
model runtime, PDF parser, encrypted packs, IVF sharding, Rayon/Tantivy paths,
database proxies, market tooling, multimodal experiments, benchmark corpora,
local caches, keys, memory databases, transcripts, and checkpoints.

These are separate experiments or future channels. They do not belong in the
small default that protects a person's daily work.

## Trust and license boundary

`synapse-core` is FSL-1.1-ALv2. The CLI, graph, and learning utility crates are
MIT. Both exact texts ship in `LICENSES/`. The proprietary `synapse-engine` is not
in the portable dependency graph or archive. `THIRD-PARTY-LICENSES.html` is
generated from the exact locked multi-target dependency closure.

Read next:

- [FEATURES.md](FEATURES.md) — exact capability boundary
- [RELEASE-NOTES.md](RELEASE-NOTES.md) — current release and upgrade notes
- [PROOF.md](PROOF.md) — reproducible verification evidence
- [RELEASE-GATES.md](RELEASE-GATES.md) — publication contract
- [MANIFEST.md](MANIFEST.md) — exact archive contents
