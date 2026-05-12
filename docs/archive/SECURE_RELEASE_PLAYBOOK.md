# Synapse — Secure Release Playbook (2026-04)

Concise field guide. Full research: `~/.claude/projects/-Users-master/memory/project_synapse_secure_distribution.md`.

## 0. Architecture Decision (do this first)

```
synapse-core (FSL-1.1-Apache, source-available)
   ├── MCP server, CLI, plumbing, schema
   └── dynamically loads ↓
synapse-engine.{dylib|so|dll|cwasm}   ← CLOSED, hardened, per-customer watermarked
   ├── ranking / fusion / learned router
   ├── embedded ML weights
   └── ed25519-signed, integrity-checked at load
```

Hot IP lives only in the engine artifact. Core is forkable; engine is not.

## 1. Hardened Build (`Cargo.toml` profile)

```toml
[profile.release-hardened]
inherits = "release"
opt-level = "z"
lto = "fat"
codegen-units = 1
panic = "abort"
strip = "symbols"
overflow-checks = false
debug = false
incremental = false
```

```bash
RUSTFLAGS="-C strip=symbols -C debuginfo=0 -C link-arg=-Wl,-x -Z randomize-layout" \
  cargo +nightly build --profile release-hardened --target x86_64-unknown-linux-musl
strip -x target/release-hardened/synapse-engine.dylib   # macOS
codesign --options runtime --timestamp -s "Developer ID" target/.../synapse-engine.dylib
```

## 2. Obfuscation (must-do)

- **Strings:** wrap every URL, license key, error msg, embedded prompt:
  ```rust
  use obfstr::obfstr;
  let url = obfstr!("https://license.synapse.dev/v1/verify");
  ```
- **Hot fns (engine only):** build engine with **Pluto-Obfuscator** (LLVM 19 patches) — CFG flatten + bogus control flow + MBA. Skip on core (slow builds).
- **Anti-debug:** `debugoff` crate, ptrace + PT_DENY_ATTACH + `IsDebuggerPresent`, plus rdtsc timing checks. On detect → corrupt internal state silently.
- **Self-integrity:** BLAKE3 of own `.text` section vs. baked Ed25519-signed hash. Mismatch → refuse to derive db key.

## 3. Licensing

- **Server:** Keygen.sh self-hosted (Hetzner, ~€10/mo). Ed25519 keypair in SoftHSM2.
- **Token:** JWT EdDSA, TTL 24-72h, 7-30d offline grace, `jti` revocation list.
- **Claims:** `customer_id, sku, hw_fp, exp, nbf, features[], grace_until`.
- **HW fingerprint:** BLAKE3(machine-uid + primary MAC + CPU brand). Premium: bind to Secure Enclave (mac) / TPM2 (linux) ECC key.
- **Verify in-binary:** `ed25519-dalek` + `jsonwebtoken` (EdDSA), pubkey via `obfstr!`.
- **Anti-rollback:** sealed last-seen ts + cross-NTP check.

## 4. Database Encryption (`brain.db`)

- **Now:** rusqlite + SQLCipher feature (`bundled-sqlcipher-vendored-openssl`).
- **Phase 2 (libsql migration):** libsql encrypted page-codec (ChaCha20).
- **Key derivation:**
  ```
  db_key = HKDF-SHA256(
    master = first_32B(ed25519_license_sig),
    salt   = blake3(machine_fingerprint),
    info   = "synapse-brain-v1"
  )
  ```
  Key never on disk. Lose license = lose db (offer enterprise key-escrow).
- **Sealed wrapper:** `.synapsedb` magic + version + AEAD + HMAC over SQLCipher pages.

## 5. Per-Customer Watermarking (traitor tracing)

- CI matrix: 1 build per active license. `build.rs` writes `{license_id, customer_hash, build_ts}` BLAKE3 blob into `.rodata`.
- Use `-Z randomize-layout` (nightly) for per-build code-shape variance.
- Leaked binary → BLAKE3 lookup → identify leaker. Ship DMCA + revoke license.

## 6. Distribution & SKU Matrix

| SKU | Form | Price | Protection |
|---|---|---|---|
| Free Cloud | Hosted SaaS, 1k q/day | €0 | server-side (RE-proof) |
| Pro Self-Host | Hardened binary + encrypted db + license refresh | €49-199/seat/mo | obfstr + Pluto + dylib + Ed25519 + watermark |
| Enterprise | BSL/FSL source + SLA + custom build | €25-100k/yr | legal moat |

**Docker:** `FROM scratch` + musl-static binary, `cosign` signed, `seccomp` default.

## 7. Legal

- **Core repo:** FSL-1.1-Apache-2.0 (Sentry-style, 2-yr delay → Apache 2.0).
- **Engine artifact:** proprietary EULA, no source shipped.
- **Enterprise:** BSL 1.1 with 4-yr Apache sunset, signed agreement.
- **Avoid:** AGPL (scares enterprise), MIT/Apache on core (gives away farm), Elastic License v2 only if you want simpler than BSL with no sunset.

**Trademark:** filing premature pre-traction. Defensively grab domains now: `synapsedb.dev`, `getsynapse.ai`, `synapse-db.com`. File compound mark ("SynapseDB") Class 9+42 via Gerben IP after €100k ARR.

## 8. GTM

1. HN Show HN — lead bench (0.023ms/q) + "single Rust binary" + FSL openness
2. r/LocalLLaMA — on-device, no cloud
3. r/rust — perf + zero-deps
4. MCP directories: Anthropic + Smithery + mcp.so
5. Newsletters: Latent Space, swyx AI Engineer, Pragmatic Engineer
6. Bench content vs Chroma/LanceDB/Qdrant (you already have data)

## 9. Pre-Launch Checklist

- [ ] `release-hardened` profile committed
- [ ] `obfstr!` audit: every URL/secret/key string wrapped
- [ ] Engine split into `synapse-engine` crate, `cdylib` + `staticlib` outputs
- [ ] Pluto LLVM build pipeline working (CI image pinned)
- [ ] Ed25519 license keypair generated, pubkey baked into engine via `obfstr!`
- [ ] Keygen.sh self-host deployed + tested
- [ ] SQLCipher integration + HKDF key derivation tested
- [ ] Anti-debug + self-integrity checks wired
- [ ] Per-customer build matrix in GH Actions
- [ ] Cosign-signed distroless Docker image
- [ ] Notarized macOS build (`notarytool`)
- [ ] FSL LICENSE file in core repo, EULA in engine release
- [ ] Defensive domains registered
- [ ] Watermark extraction tool kept private

## TL;DR — 3 sharpest moves

1. **Split: open core (FSL) + closed engine dylib (obfstr + Pluto + Ed25519 sig + per-customer watermark).**
2. **Keygen.sh self-host + Ed25519 short-TTL JWT + HW-fingerprint + Secure-Enclave/TPM seal on premium.**
3. **SQLCipher with HKDF(license_sig, hw_fp) — db key never on disk.**

**Skip in 2026:** UPX, goldberg, obfuscator-llvm Rust fork, AGPL, custom crypto, premature TM filing.
