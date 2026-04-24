# Persona #9 — Fintech Ed25519 Tamper-Flip Test

**Date**: 2026-04-23  
**Script**: `eval/tamper_test.sh`  
**Persona**: Elena, Fintech Compliance — "Ed25519 proof + tamper-evident"

## Method

Sign a `.brainpack` with a freshly generated Ed25519 keypair (`synapse keygen`), then flip exactly 1 byte at 5 different positions and attempt `restore` into a fresh DB. A secure implementation must reject all 5 variants.

## Results (measured 2026-04-23)

| Variant | Offset | Flip | Detected? | Notes |
|---|---|---|---|---|
| header-byte | 0x0004 | inside format header after magic | PASS | cbor/msgpack framing error on decode |
| payload-byte | ~25% of file | document content | FAIL | restore succeeds silently |
| signature-byte | last 32 bytes | Ed25519 sig tail | FAIL | restore succeeds silently |
| length-field | 0x0008 | typical length-prefix zone | FAIL | restore succeeds silently |
| version-magic-byte | 0x0000 | magic bytes | PASS | magic check rejects |

**Score: 2/5 PASS**

## Root Cause

`restore` today is a deserialise-and-import operation. It verifies the brainpack magic/format framing (catches offsets 0 and 4) but does **not** verify the Ed25519 signature embedded in the `snap-signed` header before deserialising. The signature is stored but not checked on restore.

## Gap

Real tamper-evidence requires: before any deserialisation, read the VK from the brainpack header, verify the Ed25519 sig over the entire payload, reject if invalid.

## Fix Required

Add `--vk <path>` flag to `restore` (or auto-detect VK from brainpack header):

```
synapse restore --vk synapse.vk tampered.brainpack
# → error: signature verification failed
```

Effort: ~1 day (50–80 LoC in `synapse-cli/src/main.rs` + `synapse-core` restore fn).  
After this fix, all 5 variants will be caught → **5/5 PASS**.

## Verdict

**PARTIAL PASS (2/5)**. Structural corruption (magic/header) is caught. Payload + signature tampering is not. This is a credible gap for a fintech compliance persona. Fix before claiming "tamper-evident" in marketing.
