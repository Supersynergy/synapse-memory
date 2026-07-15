# Synapse Agent Memory release proof

Evidence date: 2026-07-15. Candidate version: `1.1.0-rc.3`.

## Verdict

The local portable product path passes on macOS ARM64, including the renamed
package, the 15-stage product/install/recovery verifier, and a real install of the
earlier `synapse-memory-v1.1.0-rc.1` archive through the compatibility route.

The corrected six-target candidate matrix passes on merge commit
`b050e200af93660ab43feb8a0dfbe7fb9b9bcb62` in
[GitHub Actions run 29429954455](https://github.com/Supersynergy/synapse-agent-memory/actions/runs/29429954455).
Publication remains fail-closed: the release job runs only for a matching
`synapse-agent-memory-v*` tag after preflight, all six native executions,
packaging, and checksum validation succeed.

Canonical workflow:
[release-synapse-agent-memory.yml](../../.github/workflows/release-synapse-agent-memory.yml).

## Verified local gates

| Gate | Result |
|---|---|
| Portable CLI/core tests | PASS |
| Learning and explicit feedback tests | PASS |
| Rustfmt and Clippy with warnings denied | PASS |
| Exact six-target RustSec closure | 146 packages, 0 findings |
| Cargo dependency policy | PASS |
| First- and third-party license closure | 146/146 packages |
| Diff-aware Semgrep and 406-commit Gitleaks scan | PASS, 0 leaks |
| Full 15-stage product/package/install/recovery verifier | PASS on macOS ARM64 |
| Clean detached-HEAD snapshot plus complete release overlay | PASS on macOS ARM64 |
| Temporal parsing, event-range, supersession core regressions | PASS |
| Context noise, hard budget, feedback poisoning, calibration regressions | PASS |
| Verified backup-before-FTS-repair regression | PASS |
| Codex checkpoint recovery tests | PASS |
| ShellCheck and Actionlint | PASS |
| Legacy RC1 download, checksum, install, init, and doctor | PASS on macOS ARM64 |
| Cross-platform checksum sidecars | 6/6 PASS on macOS `shasum`, 0 CR bytes |

The broader experimental workspace lockfile contains 745 packages and reports
nine existing OSV advisories outside the portable closure. RC3 adds or upgrades
no dependency; its `Cargo.lock` delta changes only the ten first-party workspace
version entries. The release gate resolves the exact portable 146-package closure
independently and reports zero findings.

## Native candidate matrix

| Target | Result |
|---|---|
| macOS ARM64 | PASS |
| macOS x86-64 | PASS |
| Linux musl ARM64 | PASS |
| Linux musl x86-64 | PASS |
| Windows ARM64 | PASS, including current/legacy installer routing |
| Windows x86-64 | PASS, including current/legacy installer routing |

All six candidate archives and sidecars were downloaded unchanged from the run
artifacts. Every sidecar contains LF-only text, all six contain zero CR bytes,
and all six verify with macOS `shasum -a 256 -c`. This specifically closes the
cross-platform Windows-sidecar defect found during the RC2 online download test.

The first dispatch exposed an external Ubuntu ports outage before the Linux ARM64
build. The release bootstrap now probes HTTPS, uses signed-mirror fallback only
when needed, forces IPv4, and applies bounded timeouts and retries. The exact
post-fix matrix then passed 6/6.

The candidate matrix uses workflow dispatch, so publication is skipped by design.
The matching tag reruns the same fail-closed workflow before creating the GitHub
release.

## Reproduce the local proof

```sh
TARGET="$(rustc -vV | sed -n 's/^host: //p')"
cargo build --locked --profile release-hardened \
  --target "$TARGET" -p synapse-cli --no-default-features

SYNX_BIN="target/$TARGET/release-hardened/synx" \
  release/synapse-agent-memory/verify.sh

release/synapse-agent-memory/fresh-snapshot.sh
```

The verifier covers dependency licenses and policy, RustSec, native-binary guard,
temporal/priority memory, supersession filtering, bounded cited context, explicit
pass/fail feedback, offline freshness, verified backup-before-repair self-healing,
backup/restore, package contents, checksum install, corrupt-checksum and
unsafe-archive rejection, rollback, data-preserving uninstall, and Codex recovery.

## Claim boundary

This proof covers the portable lexical memory channel. It does not certify the
broader optional engine experiments, semantic model quality, automatic human-like
memory, multi-user SaaS, or competitor recall superiority.
