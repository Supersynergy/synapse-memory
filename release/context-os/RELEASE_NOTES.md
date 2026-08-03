# Synapse Context OS Release Notes

Version: 1.0.1-rc.1 context-os slice
Date: 2026-05-25

## Added

- `synx prime <repo>`: repo startup brief for coding agents with Git state,
  source docs, likely commands, recent/relevant memories, and next context,
  doctor, and freshness commands.
- `release/context-os/`: clean Mac/Linux onboarding that ships no maintainer
  data.
- `release/context-os/service.sh`: user-level macOS launchd / Linux systemd
  daemon setup.
- `release/context-os/package.sh`: shareable source tarball builder with guards
  against private local data, session logs, embedding caches, and brain database
  files. Local binaries are included only with `SYNAPSE_PACKAGE_INCLUDE_BIN=1`
  and produce target-labelled binary tarballs.
- `release/context-os/verify.sh`: 13-step release smoke for clean-user Context
  OS behavior, including optional install-from-tarball verification.
- `integrations/codex/`: reversible crash-safe resume hooks with append-only,
  fsynced checkpoints, atomic latest snapshots, content-minimal recovery hints,
  and prompt-injection coverage for untrusted path names.

## Verified

- Bounded context packs include `context_id`, retrieval route, hit ids, and a
  feedback hint.
- Feedback increments the learning database.
- Freshness guard works offline from local manifests via `--no-registry`.
- Doctor and safe FTS optimize run on a temporary database.
- Packaging dry-run rejects private local paths and forbidden data files.
- Extracted source package passes Cargo metadata/check and can install into a
  temporary clean home/prefix.
- Service dry-run install writes macOS launchd and Linux systemd user service
  files without loading/enabling them.
- Binary package dry-run is strict: all required binaries must exist and the
  package name includes the target label.
- Codex recovery integration passes 6 focused tests and is included in the
  source package without any local checkpoint journals.
- LongMemEval-S no-download baseline is published:
  `cargo run -p longmemeval --no-default-features -- --rerank-top 0` reports
  R@5=0.640, R@10=0.640, and 0 errors on the 50-question subset.

## Explicit Non-Claims

- This release does not claim graph/OLAP/TSDB/Surreal parity as the core product
  promise.
- This release does not bundle the maintainer's `~/.synapse/brain.db`.
- This release does not claim LoCoMo, judge-mode, model-rerank, or broad
  memory-SOTA quality beyond the published LongMemEval-S no-download baseline.
