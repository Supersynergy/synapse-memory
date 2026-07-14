# Synapse Security Model

> This document covers the optional daemon/engine surface. The canonical policy
> and supported portable release boundary are in [`../SECURITY.md`](../SECURITY.md).

Threat-model. What Synapse defends against, what it doesn't, and the safe-default configuration.

---

## TL;DR

Synapse is a **local-first, single-user daemon** by default. The attack surface is small because:

1. The daemon binds to a **unix domain socket** with mode `0600` — only the owning UID can connect.
2. There is no network listener by default. No TCP. No HTTP.
3. All data is on disk in a single SQLite file; no external service, no auth store.
4. The embedding pipeline uses locally-downloaded ONNX weights. No outbound traffic after first fetch.

If you expose Synapse beyond a single user's machine — e.g. multi-tenant, over TCP, as a hosted service — read **§6 Hardening** before deploying.

---

## 1. Assets

| Asset | Sensitivity | Where it lives |
|---|---|---|
| Stored documents (`docs.text`, `docs.meta`) | User-classified | `brain.db` |
| Embeddings | Low | `brain.db` (`docs_vec`) |
| Embedding cache | Low | `.emb-cache` (redb) |
| `.brainpack` snapshots | Same as docs | Wherever exported |
| Model weights | Public (BGE-small-en-v1.5) | `~/.cache/fastembed/` |

Documents frequently contain **secrets** (API keys in chat logs, PII in CRM memory). Treat the db file as sensitive.

## 2. Trust Boundaries

```
┌──────── trusted: process, user ───────────┐
│ synapsed daemon                           │
│ brain.db / .emb-cache / .brainpack        │
└──────────────┬────────────────────────────┘
               │ AF_UNIX (0600)    ← boundary 1
┌──────────────▼────────────────────────────┐
│ clients on same machine, same UID         │
└──────────────┬────────────────────────────┘
               │ user intent       ← boundary 2
┌──────────────▼────────────────────────────┐
│ third-party MCP tool, agent plan, etc.    │
└───────────────────────────────────────────┘
```

Boundary 1 is enforced by POSIX file perms.
Boundary 2 is the user deciding what to `put` — not Synapse's job to police that content.

## 3. Threat Model

| # | Threat | Default posture | Mitigation |
|---|---|---|---|
| T1 | Local process on same UID reads `brain.db` | Not defended | By design — UID trust |
| T2 | Another UID on same host reads the socket | Blocked | socket mode 0600 |
| T3 | Another UID on same host reads `brain.db` | Not defended by Synapse | Use FS perms (`chmod 600 brain.db`) or FileVault |
| T4 | Crash during write → corrupt db | Defended | SQLite WAL, `synchronous=NORMAL` |
| T5 | Malicious `.brainpack` import | Partially defended | BLAKE3 checksum verifies integrity (not authenticity) |
| T6 | SQL injection via `search` query | Not possible | All queries are parameterized; FTS5 MATCH tokens are FTS5-escaped by SQLite |
| T7 | Path traversal on `Snap { out }` | **⚠️ Current weakness** — `out` is a path the daemon writes | See §6: bind-mount or chroot in multi-tenant |
| T8 | DoS via huge frame | Defended | Daemon rejects frames >256 MB |
| T9 | Resource exhaustion via huge batch | Partially defended | Tokio scheduler back-pressures, but no per-conn quota |
| T10 | Model poisoning via replaced ONNX | Mitigated | fastembed pins upstream; verify via HF hash if paranoid |
| T11 | Replay of captured RPC on shared socket | N/A | 0600 UID-bound |
| T12 | Cross-session data bleed | N/A | Single-store per daemon; use separate daemons per tenant |

## 4. Implemented Protections

- **Parameterized SQL everywhere.** Grep-check: every `conn.execute` / `query_row` / `prepare` uses `params![]`. Zero string-concat into SQL. ✅
- **Frame-length cap.** Connection handler refuses any msgpack frame ≥ 256 MB before allocating. ✅
- **Content-addressed dedup.** BLAKE3(text) primary dedup key; identical text yields the same row. Removes a class of accidental duplication bugs. ✅
- **WAL + `synchronous=NORMAL`.** Crash-safe without the performance hit of `FULL`. ✅
- **BLAKE3 checksum on `.brainpack`.** Corruption detected at import time. ✅
- **No outbound network.** After first model download, daemon makes zero egress requests. ✅
- **No eval, no dynamic code load.** Daemon loads only statically-linked Rust code. ✅

## 5. Known Gaps

### 5.1 `Snap { out }` path is trusted

`Request::Snap { out: String }` lets a client write the snapshot to any path the daemon's UID can write. On a single-user machine this is fine — the client and daemon share the same trust domain. In a multi-tenant server mode, **this becomes a write-anywhere primitive**.

**Mitigation (current code):** only run the daemon for one user. **Planned fix (M7+):** `--snap-dir <path>` flag; reject `out` not inside that dir.

### 5.2 No authentication

The daemon trusts any client that can connect to the socket. That's fine on a unix socket with mode 0600, but broken over TCP.

**Planned fix (M7+):** HMAC-token preamble. Daemon starts with `--token-file <path>`; clients send `token = HMAC(shared_key, nonce)`. Rejects unauth-ed clients.

### 5.3 Plaintext at rest

`brain.db` is not encrypted. Anyone who reads the file sees your docs.

**Mitigation today:** use FileVault (macOS), LUKS (Linux), or put the file on an encrypted volume.
**Planned fix:** optional `--encrypt-key <path>` → SQLCipher drop-in.

### 5.4 Embedding cache is a side-channel

`.emb-cache` reveals *which* texts have been embedded (by BLAKE3 hash). On its own that's an inverted-index — a curious user on the same filesystem could check if a specific string has been stored by computing its BLAKE3 and doing a lookup.

**Who cares:** in shared-machine contexts (lab, CI runner), this could leak presence of sensitive strings. **Mitigation:** ensure the cache file's perms match the db.

### 5.5 No rate-limiting per connection

Tokio gives back-pressure but there's no per-client quota. A local bug could spin-put the daemon.

**Planned fix:** token-bucket on `dispatch`.

## 6. Hardening for Multi-Tenant / Exposed Deployments

If you deploy Synapse as a shared service (not the primary design, but feasible):

1. **One daemon per tenant.** Do not let two tenants share a daemon. They share a DB.
2. **Unix socket only, never TCP.** If you need TCP, front it with an auth-enforcing reverse proxy and HMAC tokens on every request. The wire protocol today has no auth.
3. **`chmod 600`** the db file and socket. Verify ownership.
4. **Seccomp / AppArmor.** Deny the daemon all syscalls except `read/write/openat/close/futex/sendto/recvfrom/accept4/bind/listen/mmap/munmap/brk/clock_gettime/epoll_*/pread/pwrite/fsync/fdatasync/unlink/rename/stat/lstat/getdents/exit`. It doesn't need more.
5. **File-system isolation.** Run under `systemd-confine` or a dedicated user. Set `WorkingDirectory` to a tenant-specific path.
6. **Rotate `.brainpack` exports off the main disk** — they replay the entire db as one unencrypted blob.
7. **Audit the BLAKE3 cache dir** — as noted in 5.4.

## 7. Incident Response

If a `brain.db` is suspected of containing a secret:

```bash
# 1. Stop daemon
pkill synapsed

# 2. Find offending rows
sqlite3 brain.db "SELECT id FROM docs WHERE text LIKE '%sk-%' OR text LIKE '%ghp_%';"

# 3. Remove rows + their vec + cache entry
sqlite3 brain.db "DELETE FROM docs WHERE id IN (...);"
# triggers keep fts+vec in sync.

# 4. VACUUM to reclaim pages
sqlite3 brain.db "VACUUM;"

# 5. Remove cached embedding (BLAKE3 of the leaked text is the redb key)
# easiest: rm .emb-cache and let it rebuild
rm .emb-cache
```

For `.brainpack` snapshots, delete the file. Export is a full DB dump — no selective redaction.

## 8. Reporting

Security issues: **security@supersynergy.de** (PGP key on keybase). We respond within 72 h. We do not run bug bounties; serious reports get credit + a merch voucher + a hall-of-fame entry in this file once fixed.

## 9. Out of Scope

- Protecting against the operating system. If root is compromised, Synapse can't help.
- Protecting against physical access. That's what disk encryption is for.
- Protecting against the model. Embeddings leak semantics. If you don't want semantic similarity to reveal sensitive info, don't vectorize it.

---

**Last reviewed:** 2026-04-19. Re-review each minor version.
