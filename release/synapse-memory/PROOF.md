# Synapse Memory release proof

Evidence date: 2026-07-14. Candidate version: `1.1.0-rc.1`.

## Verdict

The local portable product path and the six-target candidate matrix pass,
including the new temporal truth, supersession, feedback, and
backup-before-repair flow. Candidate commit `fec3440b5255397651722f5f0f81d60188d73bea`
passed [GitHub Actions run 29367084743](https://github.com/Supersynergy/synapse/actions/runs/29367084743).
Publication remains fail-closed: the release job runs only for a matching
`synapse-memory-v*` tag after preflight, all six native executions, packaging,
and checksum validation succeed.

Canonical workflow:
[release-synapse-memory.yml](../../.github/workflows/release-synapse-memory.yml).

## Verified gates

| Gate | Result |
|---|---|
| Portable CLI/core tests | PASS |
| Learning tests | PASS |
| Rustfmt and Clippy with warnings denied | PASS |
| Exact six-target RustSec closure | 0 vulnerabilities, 0 warnings |
| Cargo dependency policy | PASS |
| First- and third-party license closure | PASS |
| Full 15-stage product/package/install/recovery verifier | PASS on macOS ARM64 |
| Temporal parsing, event-range, supersession core regressions | PASS |
| Context noise, hard budget, feedback poisoning, calibration regressions | PASS |
| Verified backup-before-FTS-repair regression | PASS |
| Codex checkpoint recovery tests | PASS |
| ShellCheck and Actionlint | PASS |
| Current release-diff secret scan | PASS |
| Native macOS Apple Silicon | PASS |
| Native macOS Intel | PASS |
| Native Linux x86-64 musl | PASS |
| Native Linux ARM64 musl | PASS |
| Native Windows x86-64 | PASS |
| Native Windows ARM64 | PASS |

The candidate matrix used workflow dispatch, so publication was skipped by design.
The matching tag reruns the same fail-closed workflow before creating the GitHub
release.

## Reproduce the local proof

```sh
TARGET="$(rustc -vV | sed -n 's/^host: //p')"
cargo build --locked --profile release-hardened \
  --target "$TARGET" -p synapse-cli --no-default-features

SYNX_BIN="target/$TARGET/release-hardened/synx" \
  release/synapse-memory/verify.sh

release/synapse-memory/fresh-snapshot.sh
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
