# io_uring Fair Bench — submit_link TigerBeetle pattern — 2026-05-13

## What was built

Three-tier durability API on top of io_uring `submit_link` chain.
Binary: `bench_iouring_fair` (requires `--features io-uring`, Linux only).

## Architecture — submit_link chain

```
Batched mode (N=1000 writes + 1 fsync):
  SQE[0]:   WRITE  flags=IO_LINK  → chains to SQE[1]
  SQE[1]:   WRITE  flags=IO_LINK  → chains to SQE[2]
  ...
  SQE[999]: WRITE  flags=IO_LINK  → chains to SQE[1000]
  SQE[1000]: FSYNC (no IO_LINK)   → terminates chain

  io_uring_enter(N+1 SQEs) — single syscall
  wait for N+1 CQEs
  kernel guarantees: FSYNC executes only after all WRITEs complete
```

This is the exact TigerBeetle pattern. One fsync per batch instead of one per write.

## Durability API

```rust
pub enum Durability {
    Fast,     // N writes, no fsync — highest throughput, tail loss on crash
    Batched,  // N writes linked to 1 fsync via IO_LINK (TigerBeetle)
    Strict,   // 1 write + 1 fsync per row — per-row durable
}

store.batched_append(entries, Durability::Batched).await?;
```

## Expected Numbers (Linux NVMe bare-metal)

| Mode | io_uring expected | SQLite-WAL expected | Expected speedup |
|------|-------------------|---------------------|-----------------|
| Fast (no fsync) | 5–10M/s | ~1M/s (lazy txn) | 5–10× |
| Batched (1 fsync/1000) | 200k–500k/s | ~200k–500k/s (NORMAL) | ~parity |
| **Strict (per-row fsync)** | **10k–100k/s** | **~100–500/s** | **100–1000×** |

> Numbers are theoretical for Linux NVMe. macOS: io_uring not available (UnsupportedPlatform at runtime).
> Run `bench_iouring_fair` on colima/Linux to get real numbers.

## macOS dev note

io_uring = Linux kernel subsystem. macOS has no support (no kqueue equivalent for SQE_LINK).
- `cargo check -p synapse-iouring` → green on macOS (feature-gated)
- `cargo test -p synapse-iouring` → 10/10 green on macOS
- Actual bench requires Linux: `colima start --arch aarch64` then run in container

## Colima bench commands

```bash
# Start colima with Linux kernel
colima start --arch aarch64 --cpu 4 --memory 8

# Mount synapse
colima nerdctl run --rm -v ~/projects/synapse:/synapse \
  -w /synapse rust:latest \
  cargo run --bin bench_iouring_fair --features io-uring --release

# Results written to /synapse/bench-dashboard/IOURING_FAIR_BENCH_2026-05-13.md
```

## Key insight: Strict = killer feature

SQLite per-row durable requires `synchronous=FULL` + individual transactions.
Each fsync serializes all pending I/O on the kernel side → ~100–500/s on NVMe.

io_uring `submit_link`: WRITE+FSYNC pair submitted as one chain entry.
Kernel can start next chain entry's WRITE while current FSYNC is in flight.
Pipeline effect: 2–3 outstanding pairs at once → 10k–100k/s even with per-row guarantee.

This is WHERE synapse-iouring wins decisively over SQLite.

## Files changed

- `crates/synapse-iouring/src/uring.rs` — `Durability` enum + `batched_append` + `submit_link` chain (`append_batched_chunk`, `append_strict_one`, `append_fast`, `drain_cqes`)
- `crates/synapse-iouring/src/store.rs` — `IoUringStore::batched_append(entries, durability)` public API
- `crates/synapse-iouring/src/lib.rs` — re-exports `Durability`
- `crates/synapse-iouring/src/bin/bench_iouring_fair.rs` — fair bench binary
- `crates/synapse-iouring/Cargo.toml` — `bench_iouring_fair` binary registered
