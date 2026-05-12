# Synapse Security Audit 2026-05-10

## cargo audit (RUSTSEC)

| ID | Crate | Sev | Path | Fix |
|----|-------|-----|------|-----|
| RUSTSEC-2025-0069 | `daemonize 0.5.0` | vuln | pingora-core 0.8.0 → synapse-edge | bump pingora to ≥0.13 (drops daemonize) or pin patched |
| RUSTSEC-2024-0388 | `derivative 2.2.0` | unmaintained | pingora-core 0.8.0 → synapse-edge | bump pingora to ≥0.13 (drops derivative) |
| RUSTSEC-2026-0002 | `lru 0.12.5` | unsound (Stacked-Borrows) | mysql 26.0.1 → synapse-cms-bench | bump `mysql` crate or drop bench (cms-bench only) |

**Action plan**:
1. `cargo update -p pingora-core --precise <patched>` once 0.13 released; meanwhile pin or feature-gate `synapse-edge` behind `--features edge`
2. Move `synapse-cms-bench` mysql dep to `[dev-dependencies]` only
3. Add `cargo audit` to CI

## semgrep / smac-secscan (4 findings, all third-party or bench)

| File | Issue | Action |
|------|-------|--------|
| `bench/wp/cms_bench.py:69-70` | f-string SQL `CREATE/USE` with const → low-risk but bad pattern | parameterize |
| `tools/turbo/synapse_turbo.py:475` | `urllib.urlopen(url)` w/ dynamic url | switch to `requests` + scheme allowlist (`https://` only) |
| `wordpress-test/plugin-sqlite-database-integration/.../boot.php:137` | PHP `unserialize()` user data | **third-party WP plugin** — add to `.semgrepignore` (vendored test fixture) |
| `wordpress-test/.../class-wp-sqlite-crosscheck-db.php:19` | `unlink()` user data | same — vendored fixture |

## gitleaks

✅ Clean — no secrets in repo.

## Hardening checklist (next ship)

- [ ] daemon socket `/tmp/synapse.sock` mode 0600 + UID-only (verify on start)
- [ ] WAL crash-safe ingest (synapse-wal stub → implement)
- [ ] cargo-deny check (license + dup deps)
- [ ] CI: `cargo audit` + `cargo deny check` + `gitleaks` blocking
- [ ] sign release tarballs with sigstore/cosign
- [ ] SBOM via `cargo cyclonedx`
