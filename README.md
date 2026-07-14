# Synapse Memory

![Synapse Memory — private continuity for coding agents.](docs/assets/social-preview.png)

[![Synapse Memory CI](https://github.com/Supersynergy/synapse/actions/workflows/synapse-memory-ci.yml/badge.svg)](https://github.com/Supersynergy/synapse/actions/workflows/synapse-memory-ci.yml)
[![Release](https://img.shields.io/github/v/release/Supersynergy/synapse?include_prereleases&label=release)](https://github.com/Supersynergy/synapse/releases)
[![License: FSL-1.1-ALv2 + MIT](https://img.shields.io/badge/license-FSL--1.1--ALv2%20%2B%20MIT-orange.svg)](LICENSE-CORE.md)

> **Your agents forget. Your work — and the care behind it — shouldn't.**

Synapse Memory keeps the decisions, context, dates, and recovery breadcrumbs
behind your work in one private SQLite file. Close the chat, switch agents, or
come back next month: the next agent can recover what mattered, when it happened,
and what has since replaced it.

One native Rust CLI. No Docker. No cloud account. No API key. No LLM in the
retrieval path.

## What it gives you

| When this happens | Synapse Memory helps by | You get |
|---|---|---|
| A fresh session knows nothing | `synx prime .` reads the project and relevant memory | A compact starting brief, not another long explanation |
| A decision is buried in chat history | `remember` stores its type, priority, confidence, event time, and source | The reason behind the work stays with the work |
| A decision changes | `remember --supersedes <id>` links new truth to old truth | Agents see the active answer without erasing history |
| Full history would flood the context window | `context` applies a hard budget and filters transport noise and superseded memory | Useful context with source ids, dates, and an auditable route |
| You ask “what happened last week?” | English/German date cues, ISO dates, and `Q1`–`Q4` select event time | Capture time and event time no longer get confused |
| Package or API assumptions may be stale | `synx fresh-context --cwd . --prompt "..." --no-registry` reads local manifests and lockfiles | Version-aware context without sending the project away |
| Retrieval helped—or failed | Explicit pass/fail feedback records accepted and actually used ids | Score and memory-type calibration improves from real use, locally |
| The derived search index drifts | `doctor --fix` verifies SQLite, creates and restores a brainpack, then repairs only FTS | Self-healing stays reversible and leaves canonical memory untouched |
| Codex disconnects mid-task | The optional checkpoint adapter records minimal execution state | A careful recovery hint without storing the transcript |

**The promise:** bring back the smallest useful truth, with enough provenance to
check it.

## Install

macOS or Linux:

```sh
curl -fsSL https://raw.githubusercontent.com/Supersynergy/synapse/main/release/synapse-memory/install.sh | sh
```

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/Supersynergy/synapse/main/release/synapse-memory/install.ps1 | iex
```

The installer selects one of six native binaries, pins the Synapse Memory release,
verifies its SHA-256 sidecar, and runs `doctor`. It does not need Rust, Python,
Node, Docker, a database server, or a network service at runtime.

Release assets and checksums:
[GitHub Releases](https://github.com/Supersynergy/synapse/releases).

## Your first useful memory

```sh
BRAIN="$HOME/.synapse/brain.db"

synx -f "$BRAIN" init
synx -f "$BRAIN" remember --kind decision --priority critical \
  --occurred-at 2026-07-14 \
  "Run the release verifier before publishing."
synx -f "$BRAIN" context \
  "What must pass before the next release?" --mode coding
```

The context output includes a `context_id`, selected document ids, and the exact
feedback command. Reward only evidence that genuinely helped:

```sh
synx -f "$BRAIN" feedback context:<context_id> <doc_id> \
  --gate pass --used <doc_id>

# When the pack was not good enough:
synx -f "$BRAIN" feedback context:<context_id> --gate fail
```

Inside an existing project, start with:

```sh
synx -f "$BRAIN" prime .
```

## Built around human continuity

Synapse Memory does not try to preserve every token. It preserves the pieces a
person should not have to explain twice:

- decisions and their sources;
- known facts, fixes, research, and benchmarks;
- compact context selected for the task in front of you;
- feedback about which evidence actually helped;
- backups, integrity checks, and safe recovery state.

Memories remain inspectable. Retrieval remains deterministic in the portable
release. If semantic embeddings are not installed, explicit vector operations
fail clearly instead of pretending they worked.

## The long-term truth loop

Synapse Memory now separates four things that chat logs usually blur together:

1. **Captured at** — when the memory entered the brain.
2. **Occurred at** — when the event actually happened.
3. **Active truth** — the newest memory after explicit supersession.
4. **Usefulness** — pass/fail evidence from the context packs an agent used.

Priority is bounded: `critical` can break a close ranking tie, but it cannot turn
an unrelated note into a top result. Feedback ids must belong to the emitted pack,
which blocks accidental or malicious reward poisoning.

This is practical self-optimization, not a claim of consciousness or perfect
recall. The loop is measurable, local, reversible, and inspectable in SQLite.

## What happened to Telepathy?

The old Telepathy prototype tailed live transcript events. It was fast, but status
payloads and tool notifications could become durable memory. That transport is not
in the portable release.

Its best idea survives in safer form:

- optional Codex checkpoints carry minimal interruption state;
- raw sources can enter the corpus sidecar before promotion;
- only explicit, typed memory enters the high-value truth path;
- `[telepathy]`, notification, stale, and archived records are excluded from
  context without deleting evidence.

A future live transport must obey those same promotion and noise gates. It cannot
bypass them merely because an event arrived in real time.

## Keep Codex work across disconnects

The optional Codex adapter stores compact checkpoints before and after tool work:

```sh
python3 integrations/codex/install.py --dry-run
python3 integrations/codex/install.py install
```

Restart Codex once. A later session receives only a recent unfinished checkpoint
and an instruction to inspect Git, files, and processes before continuing.

Checkpoint data contains execution phase, Git HEAD, changed path names, tool name,
and a command hash. It excludes transcripts, command arguments, tool-output bodies,
and file contents. Remove it without touching unrelated hooks:

```sh
python3 integrations/codex/install.py uninstall
```

Full contract: [integrations/codex/README.md](integrations/codex/README.md).

## Private and fail-closed by default

- Memory lives in `~/.synapse/brain.db` unless you choose another path.
- Release archives never contain memory, checkpoints, transcripts, keys, or models.
- Every download has a SHA-256 sidecar; missing or mismatched checksums stop install.
- Upgrade keeps the previous binary for rollback.
- Uninstall leaves your memory untouched unless you delete it yourself.
- Backup, restore, database verification, BLAKE3, and Ed25519 signing are built in.
- `doctor --fix` refuses mutation after a failed SQLite check, creates a private
  pre-repair brainpack, restores and checks that pack, repairs only FTS, then
  records a health event. An interrupted repair remains visible on the next run.

Read the complete threat boundary in [SECURITY.md](SECURITY.md).

## Portable release

| Platform | Architecture | Asset format |
|---|---|---|
| macOS | Apple Silicon | `.tar.gz` |
| macOS | Intel | `.tar.gz` |
| Linux | x86-64 static musl | `.tar.gz` |
| Linux | ARM64 static musl | `.tar.gz` |
| Windows | x86-64 | `.zip` |
| Windows | ARM64 | `.zip` |

The portable channel ships local lexical retrieval, cited context, project
grounding, feedback, health checks, backup/restore, signatures, graph basics, and
CRDT federation. Heavy model runtimes, daemon/MCP transport, experimental database
engines, market tooling, and benchmark labs are deliberately outside this download.

Exact feature boundary: [release/synapse-memory/FEATURES.md](release/synapse-memory/FEATURES.md).

## Build and verify from source

Rust 1.95 is pinned in `rust-toolchain.toml`.

```sh
TARGET="$(rustc -vV | sed -n 's/^host: //p')"
cargo build --locked --profile release-hardened \
  --target "$TARGET" -p synapse-cli --no-default-features

SYNX_BIN="target/$TARGET/release-hardened/synx" \
  release/synapse-memory/verify.sh
```

The verifier covers the portable dependency and license closure, native-binary
guard, temporal typed memory, supersession, bounded cited context, pass/fail
feedback, backup-before-repair self-healing, offline freshness, backup/restore,
checksum install and rollback, data-safe uninstall, and Codex recovery.

Release details: [release/synapse-memory/README.md](release/synapse-memory/README.md) ·
[release proof](release/synapse-memory/PROOF.md) ·
[changelog](CHANGELOG.md)

## License and contributing

`synapse-core` uses FSL-1.1-ALv2 with an Apache-2.0 future grant. The portable CLI,
graph, and learning utility crates use MIT. The proprietary `synapse-engine` is not
part of the portable dependency graph or release archive.

See [LICENSE-CORE.md](LICENSE-CORE.md), [LICENSE](LICENSE),
[CONTRIBUTING.md](CONTRIBUTING.md), and [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
