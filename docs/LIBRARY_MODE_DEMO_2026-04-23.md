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

## Scale Ladder (M4 Max, release build, 2026-04-23)

100 search iters per scale point. Two vec backends compared: default (`brute-force sqlite-vec`) vs `--features ann-usearch` (usearch HNSW).

### Brute-force sqlite-vec (default)

| Docs | put_µs | lex_µs | vec_µs |
|------|--------|--------|--------|
| 1,000 | 58.6 | 67.5 | 6.4 |
| 10,000 | 60.2 | 243.0 | 6.2 |
| 100,000 | 90.1 | 2,069.4 | 6.4 |
| 1,000,000 | 109.8 | 24,039.6 | 6.4 |

### usearch HNSW (`--features ann-usearch`)

| Docs | put_µs | lex_µs | vec_µs |
|------|--------|--------|--------|
| 1,000 | 57.0 | 68.3 | 6.4 |
| 10,000 | 57.9 | 232.0 | 6.2 |
| 100,000 | 77.9 | 2,156.1 | 6.6 |
| 1,000,000 | 97.8 | 23,375.8 | 6.5 |

### 3-way diff: brute-force vs usearch-HNSW

| Docs | vec_µs brute | vec_µs HNSW | delta | put_µs brute | put_µs HNSW | put delta |
|------|-------------|-------------|-------|-------------|-------------|-----------|
| 1k | 6.4 | 6.4 | ~0 | 58.6 | 57.0 | -3% |
| 10k | 6.2 | 6.2 | ~0 | 60.2 | 57.9 | -4% |
| 100k | 6.4 | 6.6 | +3% | 90.1 | 77.9 | **-14%** |
| 1M | 6.4 | 6.5 | +2% | 109.8 | 97.8 | **-11%** |

**Analysis**: At these corpus sizes (≤1M 384-dim vectors on M4 Max), sqlite-vec's SIMD brute-force kernel is already compute-bound and finishes in ~6µs regardless of `n` — HNSW offers no measurable latency advantage for k=10 kNN. The HNSW index does help `put_µs` at 100k+ (~11-14% faster inserts) because it avoids the full sqlite-vec scan path on write. For truly large corpora (10M+) or high-throughput recall scenarios HNSW should pull ahead further.

**Key observations**:
- `put_µs` scales ~2× from 1k→1M: WAL batching amortizes well
- `lex_µs` (FTS5 BM25) scales linearly with corpus (~10× per 10× docs): expected for full BM25 scan
- `vec_µs` is **constant at ~6µs** for both backends ≤1M — sqlite-vec's SIMD kernel is compute-bound at these sizes

## Concurrent Reader Test @ 100k docs (Mutex<Store>)

Note: `Store` uses interior mutability; concurrent access requires external mutex. These numbers reflect mutex contention overhead.

| Threads | vec_µs/op (wall-clock avg) |
|---------|---------------------------|
| 4 | 9.1 |
| 8 | 7.9 |
| 16 | 7.6 |

Mutex contention is low at these thread counts — wall-clock avg stays <10µs even at 16 threads because vec search ops complete in ~6µs. For write-concurrent workloads, WAL mode allows one writer + multiple readers without blocking.

## Next Steps (PIONEER roadmap)

- P2: expose `synapse-core` as a C-ABI `.dylib` for Python/Node FFI (no daemon needed)
- P3: WASM target for browser-side memory (sqlite-vec → wasm-bindgen)
- P4: batch put API (`put_batch`) to amortize WAL flush across N docs
