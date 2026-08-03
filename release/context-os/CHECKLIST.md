# Context OS Release Checklist

## Required Gates

- [ ] `cargo fmt -p synapse-cli --check`
- [ ] `cargo check -p synapse-cli`
- [ ] `cargo test -p synapse-cli --bins`
- [ ] `release/context-os/verify.sh`
- [ ] `SYNAPSE_VERIFY_INSTALL=1 SYNAPSE_VERIFY_BUILD_PROFILE=dev release/context-os/verify.sh`
- [ ] `cargo fmt -p longmemeval --check`
- [ ] `cargo run -p longmemeval --no-default-features -- --rerank-top 0`
- [ ] `SYNAPSE_PACKAGE_DRY_RUN=1 release/context-os/package.sh`
- [ ] Binary dry-run requires `SYNAPSE_PACKAGE_INCLUDE_BIN=1`, `SYNAPSE_RELEASE_TARGET=<target>`, and all three binaries.
- [ ] `SYNAPSE_SERVICE_OS=Darwin SYNAPSE_SERVICE_DRY_RUN=1 release/context-os/service.sh install`
- [ ] `SYNAPSE_SERVICE_OS=Linux SYNAPSE_SERVICE_DRY_RUN=1 release/context-os/service.sh install`

## Product Gates

- [ ] `synx prime <repo>` works on a repo with no existing memories.
- [ ] `synx context <query> --json` includes `context_id`, `route`, hit ids, and `reward_hint`.
- [ ] `synx feedback context:<context_id> <doc_id>` increments `learn status`.
- [ ] `synx fresh-context --cwd <repo> --prompt <task> --no-registry` emits a freshness block from local manifests.
- [ ] `synx doctor --json` reports private/stale source hit counts and backup age.
- [ ] `synx doctor --fix` is safe on a temporary database.
- [ ] `python3 integrations/codex/hooks/test_checkpoint.py` passes the
      disconnect/recovery, content-minimization, prompt-injection, and installer gates.
- [ ] Extracted package contains `integrations/codex/install.py`; install and
      uninstall preserve unrelated Codex hooks.
- [ ] Package scan rejects maintainer-only paths, brain databases, embedding caches, session logs, `file-history`, and `node_modules`.
- [ ] Source package is buildable from an extracted tarball; binary package requires `SYNAPSE_PACKAGE_INCLUDE_BIN=1`, a target label, and complete binaries.

## Mac/Linux Gates

- [ ] macOS: `SYNAPSE_SERVICE_OS=Darwin SYNAPSE_SERVICE_DRY_RUN=1 service.sh install` writes a valid launchd plist without loading it.
- [ ] macOS: `service.sh install` loads `com.synapse.context-os` as a user LaunchAgent.
- [ ] Linux: `SYNAPSE_SERVICE_OS=Linux SYNAPSE_SERVICE_DRY_RUN=1 service.sh install` writes a valid systemd user unit without enabling it.
- [ ] Linux: `service.sh install` enables `synapse-context-os.service` as a user service.

## Non-Goals For This Release

- No bundled maintainer brain data.
- No default graph/OLAP/TSDB/Surreal-parity path.
- No Claude/Codex hook history in the package.
- No local Synapse checkpoint journals in the package.
- No root service install.
- No MLX/LLM extraction claim until quality baseline exists.
