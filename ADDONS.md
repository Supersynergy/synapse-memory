# Synapse Addons Roadmap — Hindsight-Parity + Moat

Vs. Hindsight: matching integration ecosystem. Vs. native moat: library-mode + sign + CRDT nobody else has.

## Tier 1 — Drop-in Hindsight replacements (competitive)

### `synapse-openclaw` (TS package)
- Wrapper compatible with `@vectorize-io/hindsight-client` API
- `retain(bankId, text) / recall(bankId, query, {budget}) / reflect()` → maps to `put/search/timeline`
- Socket or MCP-backed
- **Unique win**: Ed25519-signed memory shareable via `.brainpack`

### `synapse-opencode` (TS hooks)
- `prefetch/extraction/precompact` lifecycle hooks
- Same API surface as hindsight-integrations/opencode
- **Unique win**: local-first, sub-ms hybrid

### `synapse-claude-code`
- SessionStart hook (✅ done) + UserPromptSubmit for auto-recall
- Tool-use middleware for memory injection on every prompt
- **Unique win**: zero-IPC via library-mode if Rust bindings embedded

### `synapse-cursor` + `synapse-aider` + `synapse-paperclip`
- Same pattern: implement their memory plugin contract
- Shared core in TS + thin adapters per host

### `synapse-hermes-plugin`
- Python MemoryProvider ABC impl
- `HindsightEmbedded` API shape → synapse-socket shim
- 1:1 drop-in in `$HERMES_HOME/plugins/`

## Tier 2 — Bank/Scope system (first-class multi-tenant)

Hindsight's killer: `bankId` everywhere (per-project/per-agent/per-user memory).
Synapse has `meta.scope` string today — less ergonomic.

**Addon**: `synapse-banks` — SDK sugar
```rust
let bank = db.bank("project/eventshub");  // scopes all ops
bank.put("decision X")?;
bank.search("X")?;  // auto-filtered
```
**Impact**: UX parity with Hindsight.

## Tier 3 — MCP Streamable HTTP adapter

Hindsight uses `transport: "streamable_http"` for remote MCP. Synapse has only stdio.
**Addon**: `synapse-mcp-http` — HTTP/SSE streaming MCP server over socket
**Impact**: remote access for team brains + web UIs

## Tier 4 — SDK languages (parity)

Hindsight: Python + TS.  
Synapse today: Rust + CLI + Python-ish via msgpack.

**Addons**:
- `synapse-sdk-ts` (npm) — fetch-only or unix-socket via node
- `synapse-sdk-py` (pypi) — currently `/tmp/synapse_sock.py`, package it
- `synapse-sdk-go` — trivial, msgpack-go
- `synapse-sdk-swift` — iOS/macOS native app memory

## Tier 5 — Content adapters (mirror hindsight cookbook)

Hindsight has: sanity-blog, notion-import, github-activity, slack-threads
**Addons**:
- `synapse-sanity` / `synapse-notion` / `synapse-github-commits` / `synapse-slack` / `synapse-obsidian` / `synapse-miniflux` / `synapse-gmail` / `synapse-calendar`

Pattern: cron pulls source → normalize → `syn put-batch` → scope-tagged.

## Tier 6 — Unique synapse-only addons (moat)

- `synapse-sign-verify-cli` — team-distributed signed memory bundles
- `synapse-federate-demo` — P2P sync 2-laptop demo video
- `synapse-wasm` — browser-embedded memory (Hindsight can't)
- `synapse-mlx` — Metal-accelerated embed on M-series (5× Hindsight)
- `synapse-library-mode-crate` — `cargo add synapse` for Rust apps (nobody has this)
- `synapse-bandit-tuner` — visualize+tune self-learning ranker

## Prio matrix

| Addon | Impact | Effort | ROI |
|---|---|---|---|
| synapse-sdk-py packaged | high | 1h | ★★★★★ |
| synapse-sdk-ts | high | 3h | ★★★★★ |
| synapse-claude-code auto-recall hook | high | 2h | ★★★★★ |
| synapse-banks sugar | high | 4h | ★★★★ |
| synapse-hermes-plugin | medium | 4h | ★★★★ (targets NousResearch users) |
| synapse-openclaw drop-in | medium | 6h | ★★★★ |
| synapse-mcp-http | high | 6h | ★★★ |
| synapse-wasm | huge (unique) | 2d | ★★★★★ (moat) |
| synapse-mlx | high (perf) | 1d | ★★★★ |
| synapse-library-mode (doc+release) | high | 2h | ★★★★★ |

## First 72h sprint

1. Publish `synapse-core` on crates.io — library-mode is the killer feature
2. Package `synapse-sdk-py` on PyPI with socket client
3. Write `synapse-claude-code` hook-pack (SessionStart + UserPromptSubmit + Stop)
4. Demo video: federate 2 laptops with `.brainpack` sync

After that: Hindsight-parity adapter push.
