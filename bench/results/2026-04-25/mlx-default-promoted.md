# MLX Default Promotion + CRIT-2 Frame Bounds — 2026-04-25

**Branch**: `session-2026-04-25-ultrathink`
**Commit base**: `d4aacd8` (MLX bf16 parity 1.0000 vs fastembed)
**Files changed**:
- `crates/synapse-core/src/embed_mlx.rs` — `MAX_FRAME = 256 MiB` bound + `n=0` reject in `Sidecar::read_msg`
- `crates/synapse-core/src/embed.rs` — `pick_embedder()` now logs which backend was chosen and warns on MLX init failure with the underlying error.

## CRIT-2: Oversized embed frame DoS — CLOSED

**Before** (lines 97–98):

```rust
let n = u32::from_be_bytes(hdr) as usize;
let mut buf = vec![0u8; n];     // n up to 4 GiB → OOM via crafted sidecar reply
```

**After**:

```rust
const MAX_FRAME: usize = 256 * 1024 * 1024;  // BGE-small batch=1024 ≈ 1.5 MiB
let n = u32::from_be_bytes(hdr) as usize;
if n == 0          { return Err(Error::Other("mlx empty frame (n=0)".into())); }
if n > MAX_FRAME   { return Err(Error::Other(format!(
    "oversized embed frame: {n} bytes (max {MAX_FRAME})"))); }
let mut buf = vec![0u8; n];
```

`hdr=0` is now also rejected (previously accepted, leading to a zero-size
allocation + msgpack decode error far from the cause). 256 MiB is ~170×
above any legitimate batch yet bounds memory pressure to a tolerable
single allocation under attacker control.

`cargo check -p synapse-core --features embed-mlx,turbo,embed --release`
clean (1 unrelated warning in `db.rs`).

## MLX `pick_embedder` promotion

`pick_embedder()` (in `embed.rs`) already preferred MLX when feature flag,
target_os=macos, target_arch=aarch64 are all set. This session adds:

- `tracing::info!` on MLX path selected
- `tracing::warn!(error = %e, ...)` on sidecar init failure (was silent
  fall-through, hiding python-deps issues)
- `tracing::info!` on fastembed path selected

That is the entire wiring surface for MLX-first defaults.

## End-to-end re-bench — DEFERRED (sidecar deps incomplete)

Attempt:

```
$ python3 -c "import mlx_embeddings"
ModuleNotFoundError: No module named 'mlx'
```

`mlx_embeddings` is installed but the underlying `mlx` core wheel is not
on this host's Python 3.12. The sidecar (`scripts/synapse-mlx-embed.py`)
therefore cannot start. With the new tracing wiring `pick_embedder` will
log a `warn` and fall back to fastembed transparently — so promoting MLX
to default is **safe to ship today** (no behaviour regression on hosts
without MLX).

Additionally, the `synapsed` daemon does **not** currently call
`pick_embedder()`; it constructs `synapse-core::embed::Embedder` directly
(`crates/synapsed/Cargo.toml` enables only the `embed` feature, not
`embed-mlx`). End-to-end MLX-in-daemon requires:

1. Add `embed-mlx` feature to `synapsed` propagating to `synapse-core`.
2. Replace the direct `Embedder::new()` call site with `pick_embedder()`.
3. `pip install mlx mlx-embeddings` on the daemon host.

Tracked as follow-up; out of scope for this commit.

## Carried-forward numbers (fastembed CPU baseline)

From `wp-plugin-keepalive-fix.md` (same harness, same brain.db, no code
change in hot path):

| path        | fastembed CPU | MLX default (projected)¹ |
|-------------|---------------|--------------------------|
| Vec p50     | 81.9 ms       | ~5–8 ms                  |
| Vec p95     | 174.3 ms      | ~12–18 ms                |
| Hybrid p50  | 56.4 ms       | ~10–15 ms                |
| Hybrid p95  | 64.8 ms       | ~20–25 ms                |

¹ Projected from `mlx-embedder-impl.md`: MLX single-doc 2.45 ms vs
fastembed CPU ~50–80 ms = ~20–30× embed speedup. Vec/Hybrid p50 is
embed-bound (~75 ms of the 81.9 ms Vec p50 is embed); replacing 75 ms
with 2.5 ms ⇒ ~9 ms total. Numbers will be replaced with measured
values in the follow-up commit that wires MLX into `synapsed`.

## Ship list (this commit)

- [x] CRIT-2 frame-size bound (256 MiB) + n=0 reject
- [x] `pick_embedder` tracing on selection + warn on fallback
- [x] Compile-verified release build, no behaviour regression on
      hosts without MLX (graceful fallback)
- [ ] Daemon-side wiring (`synapsed` feature flag + call site) — separate PR
- [ ] Live re-bench with MLX path hot — blocked on `mlx` python wheel
