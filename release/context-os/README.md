# Synapse Context OS Release

Release-oriented onboarding for a clean Mac or Linux user. This package does not
include any local brain database, embeddings cache, session logs, hooks history,
or user memories.

## Promise

Synapse is a local-first Context OS for coding agents:

- `synx remember` stores typed, cited memories.
- `synx context` returns a bounded context pack with route, ids, and feedback hint.
- `synx feedback` rewards context that was actually useful.
- `synx fresh-context` adds version/API freshness from local manifests.
- `synx prime` creates a repo startup brief for a new agent session.
- `synx doctor --fix` checks health, source hygiene, backup age, and performs safe FTS optimization.
- Optional Codex hooks preserve a compact resume pointer across interrupted sessions without storing transcript or tool-output bodies.

The core product line is: best context, not biggest context.

## Install From This Repo

```bash
cd /path/to/synapse
release/context-os/install.sh
```

## Install From The Release Tarball

```bash
tar -xzf synapse-context-os-1.0.1-rc.1.tar.gz
cd synapse-context-os-1.0.1-rc.1
./install.sh
```

Defaults:

- binaries: `$HOME/.local/bin/synx`, `$HOME/.local/bin/synapsed`, `$HOME/.local/bin/synapse-mcp`
- database: `$HOME/.synapse/brain.db`
- no bundled data; the first run creates an empty local brain

Override paths:

```bash
SYNAPSE_PREFIX=/opt/synapse SYNAPSE_DB=$HOME/.synapse/context-os.db release/context-os/install.sh
```

## Verify Clean User Flow

The verifier creates a temporary project and a temporary brain database, then
exercises the release-critical Context OS commands without reading your local
`~/.synapse/brain.db`.

```bash
cd /path/to/synapse
release/context-os/verify.sh
```

Use an already-built binary:

```bash
SYNX_BIN=$HOME/.local/bin/synx release/context-os/verify.sh
```

Verify the full package install path too:

```bash
SYNAPSE_VERIFY_INSTALL=1 SYNAPSE_VERIFY_BUILD_PROFILE=dev release/context-os/verify.sh
```

## Optional User Daemon

The release works without a daemon, but a background `synapsed` keeps repeated
agent calls warm.

```bash
release/context-os/service.sh print      # dry-render service file
SYNAPSE_SERVICE_DRY_RUN=1 release/context-os/service.sh install
release/context-os/service.sh install    # macOS LaunchAgent or Linux systemd user service
release/context-os/service.sh status
```

Defaults:

- socket: `/tmp/synapse.sock`
- metrics: `127.0.0.1:9090`
- live query: `127.0.0.1:9091`

Override before install:

```bash
SYNAPSE_SOCK=/tmp/synapse-dev.sock SYNAPSE_METRICS_ADDR=127.0.0.1:19090 release/context-os/service.sh install
```

## Build A Shareable Package

```bash
SYNAPSE_PACKAGE_DRY_RUN=1 release/context-os/package.sh
release/context-os/package.sh
```

`package.sh` packages a minimal buildable Context OS source workspace, release
docs/scripts, and optional built binaries. It rejects maintainer-only paths,
session logs, embedding caches, and brain database files.

By default packages are source-only and build on the target Mac/Linux machine.
Binary packages are target-specific. Include already-built local binaries only
when you intentionally want a labelled platform package:

```bash
cargo build --release -p synapse-cli -p synapsed -p synapse-mcp --bins
SYNAPSE_PACKAGE_INCLUDE_BIN=1 \
  SYNAPSE_RELEASE_TARGET=macos-aarch64 \
  release/context-os/package.sh
```

Binary packaging is strict: `synx`, `synapsed`, and `synapse-mcp` must all be
present in `SYNAPSE_BIN_DIR` or `target/release`, and the tarball name includes
the target label.

## First Agent Session

Inside any repo:

```bash
synx -f "$HOME/.synapse/brain.db" prime .
synx -f "$HOME/.synapse/brain.db" remember --kind decision "Use Synapse context packs before major code edits."
synx -f "$HOME/.synapse/brain.db" context "current repo task" --mode coding
synx -f "$HOME/.synapse/brain.db" fresh-context --cwd . --prompt "latest package API changes"
synx -f "$HOME/.synapse/brain.db" doctor --fix
```

## Optional Codex Crash-Safe Resume

Preview and install the reversible Codex hooks:

```bash
python3 integrations/codex/install.py --dry-run
python3 integrations/codex/install.py install
```

Restart Codex once after installation. The hook keeps an append-only,
`fsync`-ed journal under `~/.synapse/checkpoints/`. It stores execution state,
Git HEAD, and changed path names—not transcript, tool output, file bodies, or
command arguments. A later `SessionStart` injects only a recent unfinished
checkpoint and tells the agent to inspect current state before replaying work.

When a returned memory was useful:

```bash
synx -f "$HOME/.synapse/brain.db" feedback context:<context_id> <doc_id>
synx -f "$HOME/.synapse/brain.db" learn calibrate
```

## Mac And Linux Notes

Mac:

- Install Rust with `rustup`.
- `$HOME/.local/bin` should be on `PATH`.
- A launchd service is optional; the release works without a background daemon.

Linux:

- Install Rust with `rustup`.
- Install a C++ runtime if your target uses dynamic ONNX Runtime.
- `$HOME/.local/bin` should be on `PATH`.

For cross-Linux builds from macOS, use the existing `cross-linux` feature and
ship `libonnxruntime.so` next to the binary. The clean release verifier does not
require private data or a daemon.

## Release Gates

Before calling this release complete:

- `release/context-os/verify.sh` passes.
- `SYNAPSE_VERIFY_INSTALL=1 SYNAPSE_VERIFY_BUILD_PROFILE=dev release/context-os/verify.sh` passes.
- `SYNAPSE_PACKAGE_DRY_RUN=1 release/context-os/package.sh` passes.
- `SYNAPSE_PACKAGE_INCLUDE_BIN=1 SYNAPSE_RELEASE_TARGET=<target> SYNAPSE_BIN_DIR=<bin-dir> release/context-os/package.sh` produces a target-labelled binary package only when all required binaries exist.
- `SYNAPSE_SERVICE_OS=Darwin SYNAPSE_SERVICE_DRY_RUN=1 release/context-os/service.sh install` passes.
- `SYNAPSE_SERVICE_OS=Linux SYNAPSE_SERVICE_DRY_RUN=1 release/context-os/service.sh install` passes.
- `cargo check -p synapse-cli` passes.
- `synx prime .` works in a repo with no existing Synapse data.
- `synx context` logs route/chosen ids and prints a feedback hint.
- `synx fresh-context` works without registry network via `--no-registry`.
- `synx doctor --json` reports docs/vectors, duplicate hashes, missing vectors, private/stale source hits, and backup age.
- `synx doctor --fix` is safe on a temporary database.

Full-repo quality gate, not bundled in the minimal Context OS tarball:

- `cargo run -p longmemeval --no-default-features -- --rerank-top 0` reports the published LongMemEval-S no-download baseline.
