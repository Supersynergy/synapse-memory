# Synapse Agent Memory release proof

Evidence date: 2026-07-15. Candidate version: `1.1.0-rc.2`.

## Verdict

The local portable product path passes on macOS ARM64, including the renamed
package, the 15-stage product/install/recovery verifier, and a real install of the
earlier `synapse-memory-v1.1.0-rc.1` archive through the compatibility route.

The renamed six-target candidate matrix is pending. Publication remains
fail-closed: the release job runs only for a matching `synapse-agent-memory-v*`
tag after preflight, all six native executions, packaging, and checksum
validation succeed.

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
| Diff-aware Semgrep and 405-commit Gitleaks scan | PASS, 0 leaks |
| Full 15-stage product/package/install/recovery verifier | PASS on macOS ARM64 |
| Clean detached-HEAD snapshot plus complete release overlay | PASS on macOS ARM64 |
| Temporal parsing, event-range, supersession core regressions | PASS |
| Context noise, hard budget, feedback poisoning, calibration regressions | PASS |
| Verified backup-before-FTS-repair regression | PASS |
| Codex checkpoint recovery tests | PASS |
| ShellCheck and Actionlint | PASS |
| Legacy RC1 download, checksum, install, init, and doctor | PASS on macOS ARM64 |

The broader experimental workspace lockfile contains 745 packages and reports
nine existing OSV advisories outside the portable closure. This candidate changes
only the ten first-party workspace version entries in `Cargo.lock`; it adds or
upgrades no dependency. The release gate resolves the exact portable 146-package
closure independently and reports zero findings.

## Native candidate matrix

| Target | Result |
|---|---|
| macOS ARM64 | PENDING |
| macOS x86-64 | PENDING |
| Linux musl ARM64 | PENDING |
| Linux musl x86-64 | PENDING |
| Windows ARM64 | PENDING |
| Windows x86-64 | PENDING |

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
