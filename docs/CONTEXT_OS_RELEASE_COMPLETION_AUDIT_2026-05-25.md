# Context OS Release Completion Audit — 2026-05-25

## Scope

Objective: ship Synapse as a local Context OS for agents, not as a broad
database-sprawl product. A new Mac/Linux user should be able to install the
release from `release/`, use it without maintainer data, and verify the core
Context OS workflow.

Core promise:

> Best context, not biggest context.

## Requirement Status

| Requirement | Status | Evidence |
|---|---:|---|
| Clean release folder exists | PASS | `release/context-os/` with README, manifest, checklist, notes, install, service, package, verify scripts |
| No maintainer data shipped | PASS | Final tarball audit rejects maintainer home paths, `brain.db`, WAL/SHM, `.emb-cache`, `.claude`, `.codex`, `node_modules`, `file-history` |
| New-user source install works | PASS | `SYNAPSE_VERIFY_INSTALL=1 SYNAPSE_VERIFY_BUILD_PROFILE=dev release/context-os/verify.sh` installs from extracted tarball into temp home/prefix/DB |
| Mac service path covered | PASS | `SYNAPSE_SERVICE_OS=Darwin SYNAPSE_SERVICE_DRY_RUN=1 release/context-os/service.sh install` writes LaunchAgent and does not load it |
| Linux service path dry-covered | PASS | `SYNAPSE_SERVICE_OS=Linux SYNAPSE_SERVICE_DRY_RUN=1 release/context-os/service.sh install` writes systemd user unit and does not enable it |
| `synx prime` exists and is verified | PASS | `release/context-os/verify.sh` gate 9 checks `prime --json` on clean project |
| Context route/id/feedback loop exists | PASS | `release/context-os/verify.sh` gates 7-8 check `context_id`, `route`, `reward_hint`, feedback, and `learn status` |
| Freshness guard has offline path | PASS | `release/context-os/verify.sh` gate 10 checks `fresh-context --no-registry` |
| Doctor/autofix guard exists | PASS | `release/context-os/verify.sh` gate 11 checks doctor JSON, `doctor --fix`, and `db-verify` |
| Root docs point to Context OS | PASS | `README.md`, `SPEC.md`, and `docs/CONTEXT_OS_PLAN_2026-05-18.md` now name `release/context-os/` as the clean release path |
| Real Linux service enablement | NOT PROVEN | Needs `systemd --user enable --now` on an actual Linux host |
| Platform-specific binary tarballs | POLICY SET | Source package remains default; binary packages require `SYNAPSE_PACKAGE_INCLUDE_BIN=1`, a target label, and complete binaries |
| LongMemEval-S quality baseline | PASS | `cargo run -p longmemeval --no-default-features -- --rerank-top 0` on 50-question subset: R@5=0.640, R@10=0.640, 0 errors; see `docs/LONGMEMEVAL_RESULTS_2026-05-25.md` |
| LoCoMo / judge / model-rerank quality baseline | NOT PROVEN | Do not claim broad memory-SOTA quality until additional benchmarks are published |
| Full SPEC-vs-reality cleanup | PARTIAL | Release path aligned; broad engine claims still need broader audit/refactor beyond Context OS slice |

## Verification Commands

Current green gates:

```bash
cargo fmt -p synapse-cli --check
cargo check -p synapse-cli
cargo test -p synapse-cli --bins
cargo fmt -p longmemeval --check
cargo run -p longmemeval --no-default-features -- --rerank-top 0
release/context-os/verify.sh
SYNAPSE_VERIFY_INSTALL=1 SYNAPSE_VERIFY_BUILD_PROFILE=dev release/context-os/verify.sh
release/context-os/package.sh
shasum -a 256 -c release/dist/synapse-context-os-1.0.1-rc.1.tar.gz.sha256
```

Package audit:

```bash
tar -xzf release/dist/synapse-context-os-1.0.1-rc.1.tar.gz -C /tmp/synapse-audit
rg -n '/Users/master|/home/master' /tmp/synapse-audit/synapse-context-os-1.0.1-rc.1
```

The package audit must return no hits.

## Remaining Completion Gates

1. Run and document real Linux user-service install on a Linux host:

   ```bash
   release/context-os/install.sh
   release/context-os/service.sh install
   systemctl --user status synapse-context-os.service --no-pager
   ```

2. Optional: build real platform binary tarballs after target-specific release
   builds:

   ```bash
   cargo build --release -p synapse-cli -p synapsed -p synapse-mcp --bins
   SYNAPSE_PACKAGE_INCLUDE_BIN=1 SYNAPSE_RELEASE_TARGET=macos-aarch64 release/context-os/package.sh
   ```

3. Extend memory-quality baseline:

   - optional ONNX reranker run with explicit model/cache state;
   - judge-mode run for LongMemEval protocol parity;
   - LoCoMo or another multi-session benchmark before broad memory-SOTA claims.

4. Continue SPEC-vs-reality cleanup:

   - demote unsupported engine claims from root docs;
   - move experimental Graph/OLAP/TSDB/SQL-wire material out of first-run docs;
   - keep Context OS workflow as the primary product surface.
