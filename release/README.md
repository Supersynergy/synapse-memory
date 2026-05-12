# Synapse Secure Release — Operator Runbook

One-page checklist. Each artifact in this directory is self-contained and cross-linked here.

## Files

- `Cargo.toml.hardened` — paste into workspace root; defines `release-secure` profile.
- `build-secure.sh` — multi-target hardened build with per-customer watermark.
- `client-verify.rs` — embed in client crate; verifies JWT, hw_fp, anti-debug.
- `brain-encrypt.rs` — SQLCipher key derivation + plaintext migration helper.
- `license-server/` — axum service: `/activate`, `/refresh`, `/revoke`.
- `CI-watermark.yml` — GitHub Actions matrix, per-customer build + S3 signed URL.

## Cut a release for a customer

1. Issue license: `sqlite3 licenses.db "INSERT INTO licenses(license_key,customer_id,created_at) VALUES('LK-...','cust_x',unixepoch())"`.
2. Trigger CI: `gh workflow run synapse-secure-release.yml -f customers='["cust_x"]'`.
3. Verify the matrix completes 4 targets green; download artifact or use the signed URL printed in the job summary.
4. Hand customer: signed URL + their `license_key`. They run `synapse activate --key LK-...` which calls `/activate` and caches the JWT under `$XDG_DATA_HOME/synapse/license.jwt`.
5. Confirm watermark: `objcopy -O binary --only-section=.synwm dist/cust_x/<triple>/synapse - | strings` (linux) or `xattr -p com.synapse.watermark dist/cust_x/<triple>/synapse | base64 -d` (mac).

## Rotate signing key

1. Generate new key: `minisign -G -p keys/ed25519.pub.new -s keys/ed25519.key.new`.
2. Add new pubkey PEM to `client-verify.rs::embedded_pubkey_pem()` as second accepted key (verify either-or for one release window).
3. Ship transition release; wait until all customers active on it (check `licenses.last_seen`).
4. Remove old pubkey from `client-verify.rs`; rotate `ED25519_PRIVATE_KEY` GitHub secret to new key.
5. Cut next release; old binaries no longer verify and refuse new JWTs.

## Revoke a license

```
curl -X POST https://license.synapse.example/revoke \
  -H 'content-type: application/json' \
  -d '{"license_key":"LK-...","admin_tok":"$LIC_ADMIN_TOKEN"}'
```

Effect: client refresh fails within 72h (JWT TTL); next launch hits `/activate` and is denied. To force-kill sooner, ship a CRL fetch in client `verify_license` (out of scope here).

## Rebuild for a new customer

1. `gh workflow run synapse-secure-release.yml -f customers='["cust_new"]'` — no code change required.
2. Watermark auto-bakes from `CUSTOMER_ID` env in `build-secure.sh`.
3. Each customer gets a unique sha256 (different watermark section), so leaks are traceable.

## Pre-flight checklist

- [ ] `Cargo.toml` includes `release-secure` profile.
- [ ] `RUSTFLAGS=--remap-path-prefix` set in CI to scrub local paths.
- [ ] `ED25519_PRIVATE_KEY`, `APPLE_IDENTITY`, `AWS_*`, `RELEASE_BUCKET` present in repo secrets.
- [ ] License-server `LIC_ED25519_PEM` and `LIC_ADMIN_TOKEN` injected via secret manager.
- [ ] Plaintext `brain.db` migrated via `migrate_from_plaintext` before first encrypted run.
- [ ] Client embeds correct pubkey PEM; build.rs replaces the placeholder.
- [ ] `build-secure.sh` exits 0 locally with a test `CUSTOMER_ID`.

## Threat coverage (short)

- Static binary analysis: stripped + LTO + panic=abort + obfstr endpoints.
- Runtime tamper: ptrace/sysctl P_TRACED checks in `client-verify`.
- Key extraction: brain.db requires both license signature + hw_fp; neither alone suffices.
- License sharing: hw_fp pin + 72h JWT + revoke endpoint.
- Leak attribution: per-customer watermark section + per-build sha256.
