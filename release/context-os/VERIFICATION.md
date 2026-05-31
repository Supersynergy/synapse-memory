# Context OS Release Verification

Date: 2026-05-25

## Verified In This Workspace

| Gate | Result | Evidence |
|---|---:|---|
| Shell syntax | PASS | `bash -n release/context-os/install.sh release/context-os/service.sh release/context-os/package.sh release/context-os/verify.sh` |
| Package dry run | PASS | `SYNAPSE_PACKAGE_DRY_RUN=1 release/context-os/package.sh` staged a minimal buildable source workspace, release docs/scripts, sample data, `FILES.txt`, and `SHA256SUMS` |
| Binary package policy | PASS | `release/context-os/verify.sh` dry-runs `SYNAPSE_PACKAGE_INCLUDE_BIN=1` with a fake complete bin dir and asserts target-labelled package output |
| Service dry install | PASS | `SYNAPSE_SERVICE_OS=Darwin SYNAPSE_SERVICE_DRY_RUN=1 release/context-os/service.sh install` wrote a user LaunchAgent without loading it |
| Linux service dry install | PASS | `SYNAPSE_SERVICE_OS=Linux SYNAPSE_SERVICE_DRY_RUN=1 release/context-os/service.sh install` wrote a systemd user unit without enabling it |
| Source package | PASS | `release/context-os/package.sh` wrote `release/dist/synapse-context-os-1.0.1-rc.1.tar.gz` |
| Package content scan | PASS | Extracted tarball contains source needed for Context OS install plus release docs/scripts/sample metadata; no maintainer home paths, `brain.db`, `.emb-cache`, session dirs, `node_modules`, or `file-history` |
| Package checksum sidecar | PASS | `release/context-os/package.sh` writes `release/dist/synapse-context-os-1.0.1-rc.1.tar.gz.sha256` next to the tarball |
| Package manifest/build smoke | PASS | Extracted tarball: `cargo metadata --no-deps --format-version 1` and `cargo check -p synapse-cli --bins` |
| Clean-user smoke | PASS | `release/context-os/verify.sh` completed 12/12 |
| Package install smoke | PASS | `SYNAPSE_VERIFY_INSTALL=1 SYNAPSE_VERIFY_BUILD_PROFILE=dev release/context-os/verify.sh` built and installed from the extracted tarball into a temporary home/prefix |
| Rust format | PASS | `cargo fmt -p synapse-cli --check` |
| Rust check | PASS | `cargo check -p synapse-cli` |
| CLI binary tests | PASS | `cargo test -p synapse-cli --bins` ran 2/2 tests |
| LongMemEval-S no-download baseline | PASS | `cargo run -p longmemeval --no-default-features -- --rerank-top 0` on 50-question subset: R@5=0.640, R@10=0.640, 0 errors |
| Secret scan, touched scope | PASS | No OpenAI/Anthropic/Stripe/AWS key pattern in `release/context-os`, `release/README.md`, `docs/CONTEXT_OS_PLAN_2026-05-18.md`, or `crates/synapse-cli/src/main.rs` |

## Clean-User Smoke Coverage

`release/context-os/verify.sh` uses a temporary `$HOME`, temporary project, and
temporary `brain.db`; it does not read the maintainer's local Synapse data.

Covered commands:

- `synx init`
- `synx remember --kind decision --no-embed`
- `synx put --kind fact --source release-smoke --no-embed`
- `synx context --json`
- `synx feedback`
- `synx learn status`
- `synx prime --json`
- `synx fresh-context --no-registry`
- `synx doctor --json`
- `synx doctor --fix`
- `synx db-verify`

The doctor smoke asserts:

- `quick_check == ok`
- `private_source_hits == 0`
- `stale_or_generated_source_hits == 0`
- `backup_age_seconds == null` for a fresh temporary DB with no backup yet

Covered release tooling:

- `install.sh` syntax
- extracted-package `install.sh` with temporary `$HOME`, `$SYNAPSE_PREFIX`, and `$SYNAPSE_DB`
- `service.sh` syntax and dry-run install for macOS launchd and Linux systemd user services
- `package.sh` syntax and private-data guard
- binary package target label and complete-binary requirement
- `verify.sh` self-check

## Still Required Before Declaring The Whole Thread Goal Complete

- Verify `systemd --user enable --now` on an actual Linux host.
- Optional real platform binary tarballs are policy-defined but not built in this workspace.
- Finish broader build-hygiene work from `docs/SPEC-VS-REALITY-2026-05-04.md`.
- Extend quality coverage with LoCoMo, LongMemEval judge mode, and optional model-rerank runs before claiming broad memory-SOTA quality.
