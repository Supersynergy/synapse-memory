# Release gates

No public tag until every P0 box is backed by a log or artifact.

## P0: release blockers

- [ ] Repository visibility and license boundary explicitly approved.
- [ ] Workspace version, package version, tag, archive metadata, and `synx --version`
      agree. Canonical tag family remains `ctxos-v*`; do not create another family.
- [ ] Dirty worktree resolved into intentional commits; no user database, key,
      checkpoint, session, cache, model, or local absolute path in the release.
- [ ] `cargo fmt --check`, portable tests, clippy, portable RustSec closure, deny,
      and secret scan pass.
- [ ] Six native jobs build and execute: macOS/Linux/Windows, x64 and ARM64.
- [ ] Every archive contains a native `synx` binary, build metadata, README, license,
      and SHA-256 sidecar. Script wrappers are rejected.
- [ ] Clean install, add, context, feedback, backup/restore, doctor, and uninstall
      pass on every target without Rust, Python, Node, Docker, or an API key.
- [ ] Upgrade preserves an existing `~/.synapse/brain.db`; uninstall preserves data
      unless the user explicitly requests purge.
- [ ] Windows TCP federation compiles; Unix socket requests fail with a useful
      message instead of a compile error.
- [ ] Forced disconnect E2E proves Codex resume without blind mutation replay.

## P1: claim gates

- [ ] Reproducible LoCoMo/LongMemEval comparison against current Mem0, Graphiti,
      Letta, and OpenMemory versions.
- [ ] Context budget adherence and citation fidelity measured on coding tasks.
- [ ] Install time, binary size, first query, p50/p95, peak RSS, and disk growth
      published for every target.
- [ ] Semantic channel has deterministic model/version pinning, offline cache docs,
      license review, and graceful lexical fallback.
- [ ] Temporal supersession and provenance tests pass before claiming temporal memory.

## P2: distribution

- [ ] Public README points to this folder as the sole end-user release path.
- [ ] Legacy `v*` and MCP-only release workflows are labelled frozen or removed.
- [ ] Homebrew and WinGet/Scoop manifests consume the same checksummed archives.
- [ ] `cargo binstall` metadata is correct; `cargo install` remains a fallback, not
      the primary user path.
- [ ] Signed macOS artifacts and Windows Authenticode are added when distribution
      volume justifies certificate cost.

## Current verified state

### rc.2 evidence — 2026-09-13, commit `0c46bc7c`, tag `ctxos-v1.0.1-rc.2`

- GitHub Actions run `34774802769` green end-to-end: preflight (script syntax,
  locked fetch, license closure, deny, RustSec, portable tests+clippy) plus all
  six native targets (aarch64/x86_64 × apple-darwin/pc-windows-msvc/
  unknown-linux-musl) and the `release` publication job.
- Published release carries 13 assets: 6 archives + 6 `.sha256` sidecars +
  `SHA256SUMS`. Downloaded `synapse-memory-aarch64-apple-darwin.tar.gz`,
  checksum verified (`shasum -c` OK), binary reports `synapse 1.0.1-rc.2`.
- Local `verify.sh` re-run on the rc.2 portable binary: all 15 stages PASS
  (licenses, deny, RustSec, native binary, init/typed memory, cited context,
  feedback loop, offline freshness/doctor, backup/restore, packaging,
  checksummed install, safe uninstall, Codex disconnect recovery).
- Workspace `cargo nextest`: 802/802 pass. `cargo clippy --workspace
  -D warnings` clean. `cargo fmt --check` clean.
- New in rc.2: daemon-first CLI retrieval (synapsed unix socket → local store
  fallback, `--no-daemon`/`SYNAPSE_NO_DAEMON`), launchd auto-install in
  `synx onboard`, doctor `daemon=` status, retrieval-route Thompson bandit in
  `context_pack` (`route`/`route_selected_by` in manifest), `maintain --json`
  `open_ms`/`scan_ms`/`merge_ms` telemetry.
- Fixes landed: aarch64-musl `c_char` typedef (was hardcoded `i8`), musl
  `u_intN_t` via `-D` aliases (keeps blake3/zstd `.S` units compiling), Windows
  smoke `-notmatch` array-filter bug (`Out-String`), release job artifact
  ordering (checkout before `download-artifact`; default clean wiped `dist/`),
  portable-clippy gating for macOS-only daemon helpers.

### Earlier verified state (rc.1)

- Portable `synapse-cli --no-default-features` checks and runs locally on macOS
  ARM64; its six-target union is 146 resolved packages.
- `fresh-snapshot.sh` rebuilt and passed all 15 stages from detached clean `HEAD`
  plus the explicit release overlay. This exposed and fixed the previously
  gitignored `Cargo.lock`; the lockfile is now a required release file.
- Installers are pinned to `VERSION`/`ctxos-v1.0.1-rc.1`; they never follow the
  ambiguous repository-wide `latest` endpoint or a legacy `v*` asset.
- Current 3.19 MiB portable-CPU hardened binary passed all 15 local release stages on
  2026-07-13: dependency licenses/policy/RustSec, native-binary guard,
  init/remember/context, feedback, offline freshness/doctor, backup/restore,
  package, checksum install, corrupt-checksum and checksummed-symlink archive
  rejection, data-preserving uninstall, and Codex disconnect recovery.
- Portable CLI/core tests (9), learning tests (10), Codex recovery tests (7),
  clippy with warnings denied, rustfmt, ShellCheck, Actionlint, scoped Semgrep,
  current release-diff Gitleaks, and full-history Gitleaks pass locally.
- Workspace builds now default to Rust's portable CPU baseline. A verbose clean
  target build proved zero `target-cpu=native` compiler invocations; native CPU
  tuning is an explicit benchmark-only opt-in.
- Canonical memory CI/release actions are pinned to full commit SHAs. The two ARM64
  runner labels are current GitHub-hosted public-preview labels; native execution
  remains a required remote gate. Runner source:
  https://docs.github.com/en/actions/reference/runners/github-hosted-runners
- Portable add/context uses lexical retrieval and clearly rejects explicit vector
  queries without a semantic build.
- Windows source reached the native C/SQLite toolchain boundary from macOS; actual
  MSVC compile/link/run and all remote native jobs remain pending until CI runs.
- The active maintainer `~/.local/bin/synx` may be a private Python routing wrapper;
  package scripts must use an explicit Rust build artifact and reject shebang files.

## Portable security gate — resolved 2026-07-13

`release/synapse-memory/audit.sh` resolves the exact Cargo feature graph and
intersects it with the current RustSec database. Result: zero vulnerabilities and
zero warnings in the portable closure.

- `anyhow` updated to `1.0.103`.
- `quick-xml` updated to `0.41.0`.
- `pdf-extract`/`lopdf` updated, then PDF parsing excluded from portable features.
- Rayon, crossbeam, Tantivy, proprietary engine, sharding, and `age` encryption
  removed from the portable graph rather than advisory-allowlisted.

The full research monorepo still has advisories in non-shipped feature paths. Each
future channel needs its own resolved-closure audit before release.

## Local footprint proof — macOS ARM64

100 records and 100 cited-context queries, each through a fresh CLI process:

- binary: 3,342,768 bytes; SHA-256
  `d5f477570545afa4aaf663c3ae9edbf2dc16970e4af27935446191c1f4179645`
- init: 7.39 ms
- remember p50/p95: 5.37/6.01 ms
- first context: 6.95 ms; warm context p50/p95: 5.78/6.56 ms
- one context peak RSS: 4.61 MiB; database after 100 records: 204,800 bytes

Source: `evidence/macos-aarch64-local.json`. These numbers prove the local lexical
artifact only; six-target and competitor comparisons remain separate gates.

## License boundary — metadata resolved, owner publish gate remains

- `synapse-core` metadata and exact terms now agree on `FSL-1.1-ALv2`.
- CLI, graph, and learning utility crates remain MIT.
- Portable Cargo graph proves `synapse-engine` absent; its proprietary terms do not
  enter the memory artifact.
- Archives contain exact FSL and MIT first-party texts under `LICENSES/`.
- `cargo-about` report coverage and `cargo-deny -D warnings` pass for the exact
  six-target portable graph; the report ships in every archive.

Repository variable `SYNAPSE_RELEASE_LICENSE_APPROVED=true` remains a deliberate
human STOP before publication. It confirms the owner accepts this already-recorded
FSL/MIT product boundary; it does not mask a metadata conflict.
