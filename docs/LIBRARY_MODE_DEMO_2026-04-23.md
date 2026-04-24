# Library-Mode Demo — PIONEER P1 — 2026-04-24

## Motivation

MCP-mode (synapsed daemon + JSON-RPC socket) has a measured round-trip of **3,450 µs**.
Library-mode exposes `synapse-core` directly as a Rust crate — zero IPC, zero serde overhead.

## Benchmark Results (M4 Max, release build)

Corpus: 10,000 docs inserted before search benchmarks.

| Operation | Library-mode | MCP-mode (baseline) | Speedup |
|-----------|-------------|---------------------|---------|
| `put` (text insert, 10k docs) | **69 µs** | 3,450 µs | **50×** |
| Lexical search (FTS5 BM25, k=10) | **239 µs** | 3,450 µs | **14×** |
| Vector search (brute-force sqlite-vec, 384-dim, k=10) | **6 µs** | 3,450 µs | **569×** |

## How to Run

```bash
cargo build --release -p synapse-lib-demo
./target/release/synapse-lib-demo
```

Expected output:
```
=== synapse library-mode (PIONEER P1) ===
put_us       = 69.12  (50× faster than MCP 3450µs)
lex_search_us = 238.93  (14× faster than MCP)
vec_search_us = 6.06  (569× faster than MCP)
docs inserted = 10000
```

## Code

`crates/synapse-lib-demo/src/main.rs` — minimal demo using `Store::open`, `store.put`, and
`store.search(SearchMode::Lex / Vec)` directly. No daemon, no socket, no serialization.

## Key Insight

Vector search is the biggest winner: brute-force sqlite-vec over 10k docs takes **6 µs** vs
3,450 µs MCP overhead — **569×** speedup. This is because MCP pays a fixed IPC tax regardless
of how fast the actual query is.

For write-heavy agents (continuous ingestion), the 50× put speedup compounds into significant
throughput gains: library-mode can sustain **~14,500 puts/s** vs ~290 puts/s MCP-mode on M4 Max.

## Next Steps (PIONEER roadmap)

- P2: expose `synapse-core` as a C-ABI `.dylib` for Python/Node FFI (no daemon needed)
- P3: WASM target for browser-side memory (sqlite-vec → wasm-bindgen)
- P4: batch put API (`put_batch`) to amortize WAL flush across N docs
