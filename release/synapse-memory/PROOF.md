# Synapse Memory release proof

Evidence date: 2026-07-13. Candidate version: `1.0.1-rc.1`.

## Verdict

The portable package is locally release-candidate ready. It must not be described
as six-platform verified until the native GitHub matrix succeeds.

## Reproducible local proof

| Gate | Result |
|---|---|
| Detached clean `HEAD` plus explicit release overlay | `fresh-snapshot.sh`: 14/14 PASS |
| Product/package/install/recovery contract | `verify.sh`: 14/14 PASS, including bad-checksum and symlink-archive rejection |
| Exact portable six-target dependency union | 146 Cargo packages |
| RustSec over that exact union | 0 vulnerabilities, 0 warnings |
| Cargo deny | advisories, bans, licenses, and sources PASS with warnings denied |
| License payload | closure check PASS; 147 conservative report entries, no missing package |
| Rust tests | 9 portable CLI/core + 10 learning PASS |
| Codex recovery tests | 7 PASS, including fsync journal permissions and prompt-injection isolation |
| Static checks | rustfmt, Clippy `-D warnings`, ShellCheck, and Actionlint PASS |
| Release-scope Semgrep | 0 findings |
| Secrets | current diff, release overlay, and full Git history PASS |
| Release-excellence gate | 11 pass, 1 manual social-preview warning, 0 fail |
| CPU portability | verbose target build contained 0 `target-cpu=native` compiler flags |

Canonical local command:

```sh
TARGET="$(rustc -vV | sed -n 's/^host: //p')"
cargo build --locked --profile release-hardened \
  --target "$TARGET" -p synapse-cli --no-default-features
SYNX_BIN="target/$TARGET/release-hardened/synx" \
  release/synapse-memory/verify.sh
release/synapse-memory/fresh-snapshot.sh
```

## Measured macOS ARM64 artifact

- binary: 3,342,768 bytes (`3.19 MiB`)
- SHA-256: `d5f477570545afa4aaf663c3ae9edbf2dc16970e4af27935446191c1f4179645`
- remember p50/p95: `5.37/6.01 ms`
- cited context p50/p95: `5.78/6.56 ms`
- peak RSS for one context command: `4.61 MiB`
- database after 100 records: `204,800 bytes`

Full machine-readable evidence: `evidence/macos-aarch64-local.json`.

## Deliberate external STOPs

1. Commit only the intended release delta and keep unrelated dirty work out.
2. Set repository variable `SYNAPSE_RELEASE_LICENSE_APPROVED=true` after owner review.
3. Run all six native jobs: macOS, Linux, Windows; ARM64 and x64.
4. Confirm every native archive install, checksum, smoke, and uninstall in CI.
5. Make the repository/public distribution decision and upload the social preview.

The broader research monorepo's full Semgrep sweep reports a pre-existing backlog
outside the portable package, primarily mutable action tags and experimental or
benchmark code. Canonical Synapse Memory workflows are SHA-pinned and the release
scope passes a zero-finding scan. Treat repository-wide cleanup as a separate
public-source gate; do not misstate it as solved by the binary-package audit.

Competitor recall superiority is also unproven. The current defensible claim is a
small, deterministic, offline coding-agent continuity path—not “best AI memory.”
