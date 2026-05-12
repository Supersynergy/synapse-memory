# Schema-Dim Refactor Plan — Unblock Embedder Ladder

**Status**: blocker fully traced; refactor sized at ~1.5d (multi-file, requires careful const→runtime migration)

## Problem

`crates/synapse-core/src/types.rs:3`:
```rust
pub const EMBED_DIM: usize = 384;
```

40+ references across `db.rs`, `shard.rs`, `sota.rs`, `synapse-cli/main.rs`, `turbo_tests.rs`. Mix of:

| Use-site type | Examples | Const-required? |
|---|---|---|
| Stack array size | `[f32; EMBED_DIM]` (cli:440, shard.rs:33) | ✅ YES — blocks fn-conversion |
| `Vec::with_capacity(EMBED_DIM)` | db.rs:723, 759 | ❌ runtime OK |
| Length-validation `e.len() != EMBED_DIM` | db.rs:333, 409, 822 | ❌ runtime OK |
| Schema declaration `vec0(emb[EMBED_DIM])` | db.rs:100, 686, 895 | ❌ runtime OK (sqlite-vec accepts string) |
| Byte-decode `bytes.len() != EMBED_DIM * 4` | db.rs:722, 752 | ❌ runtime OK |

**Bottom line**: 4 stack-array uses (`shard.rs::centroid`, `cli::main` query-array) force const, blocking simple fn-conversion.

## Three viable approaches (ROI-sorted)

### Option A — Cargo feature flags (1d, ship-safest)

```toml
[features]
embed-384 = []  # default, BGE-small/Arctic-XS/S, MTEB ≤60
embed-768 = []  # Arctic-M / Nomic-1.5, MTEB ≤62.5
embed-1024 = [] # Mxbai-large / Arctic-L, MTEB ≤64.7
```

```rust
#[cfg(feature = "embed-1024")] pub const EMBED_DIM: usize = 1024;
#[cfg(feature = "embed-768")] pub const EMBED_DIM: usize = 768;
#[cfg(all(not(feature = "embed-1024"), not(feature = "embed-768")))]
pub const EMBED_DIM: usize = 384;
```

- ✅ minimal code change (1 file)
- ✅ no array-type refactor needed
- ⚠ requires rebuild per dim
- ⚠ binary distribution = need 3 builds OR one default + docs

### Option B — Generic const-param `<const D: usize>` (2d, cleanest)

Make `Store<const D: usize>`, `Embedder<const D: usize>`, propagate. Very Rust-idiomatic but cascades through every API surface. Major version bump (1.x → 2.x) candidate.

### Option C — Heap vectors `Vec<f32>` everywhere (1.5d, slower)

Replace `[f32; EMBED_DIM]` with `Vec<f32>`. Adds heap-alloc to hot paths. Bench shows ~2-5% throughput hit on dot-products (SimSIMD already takes slices, so fine for kernels but not for stack fast-path).

## Recommended sequence

1. **Today**: ship Option A (feature flag) — 4-hour PR, tag `v1.1.0-arctic-m`
2. **Re-run LongMemEval** with Arctic-M build → measure R@5 delta vs 0.60 baseline (expect +0.06–0.10)
3. **Next quarter**: revisit Option B if multi-dim-per-process needed (e.g. multi-tenant SaaS where corpora have different embedders)

## What's already in place

- ✅ `SYNAPSE_EMBED_MODEL` env-var (commit `6ea661f`) — fastembed loader is dim-aware via select_model()
- ✅ Arctic-M model auto-downloads to `.fastembed_cache/` on first call
- ✅ Hard error at PUT-time with helpful message (`expected 384, got 768`)

Just need the schema-side to follow.

## Acceptance criteria

After Option A ships:

```bash
cargo build --release -p longmemeval --features rerank,embed-768
SYNAPSE_EMBED_MODEL=arctic-m \
  ~/projects/synapse/target/release/longmemeval --embed --limit 30 --rerank-top 20
# Expect: Recall@5 ≥ 0.65, no dim-mismatch errors
```

Then LightGBM LambdaMART training on cumulative click-log → push toward 0.80+.
