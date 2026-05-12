# TigerBeetle vs Synapse — Transfer Bench 2026-05-12

**Hardware**: MacBook Pro M4 Max, 128 GB RAM, 8 TB SSD
**Workload**: 1 000 000 transfers (sequential batches)
**TigerBeetle**: v0.17.2, cluster=0, single replica, cache-grid default (~3 GiB alloc)
**Synapse**: v2.1-m4max, SQLite WAL, synchronous=OFF, FTS5+blake3-dedup, no embeddings

---

## Results

| Metric            | TigerBeetle             | Synapse put_batch_fast  |
|-------------------|-------------------------|-------------------------|
| Workload          | 1M financial transfers  | 1M JSON transfer-docs   |
| Batch size        | 8 189 tx (auto)         | 8 192 docs              |
| Total time        | 11.3 s                  | 22.3 s                  |
| **Throughput**    | **88 499 tx/s**         | **44 867 tx/s**         |
| Batch latency p50 | 28 ms                   | 152 ms                  |
| Batch latency p99 | 267 ms                  | 624 ms                  |
| Batch latency p100| 310 ms                  | 2 120 ms                |

---

## Caveats (PROMINENT)

### TigerBeetle advantages
- **Strict-Serializable** ACID, deterministic execution, cluster-replicated
- Custom LSM ("TigerStyle"), purpose-built for financial double-entry accounting
- Native account + transfer schema — zero serialization overhead
- Safety model: all faults handled (hardware failures, bit-rot, network splits)
- Benchmark is its native workload — no overhead from generality

### Synapse disadvantages here
- SQLite WAL ≠ strict-serializable; synchronous=OFF trades durability for speed
- Each doc requires blake3 hash + dedup check per row (FTS5 trigger)
- General-purpose: stores arbitrary text JSON, not typed 128-bit account IDs
- Latency spikes (p100 = 2 120 ms) from occasional SQLite WAL checkpoint

### Synapse advantages TigerBeetle lacks
- Hybrid BM25 + vector search (FTS5 + sqlite-vec)
- Embedding pipeline (fastembed, SimSIMD 71× SIMD kernels)
- CRDT merge, Ed25519 signing, multi-modal retrieval
- Schema-free: arbitrary JSON metadata, text, graphs
- No server process, single-file library mode (0 deps at runtime)
- Supports 113k docs @ 8 ms hybrid search (memory-resident index)

---

## Fairness Statement

This is NOT an apples-to-apples comparison. TigerBeetle solves a completely different problem:

| Dimension          | TigerBeetle                    | Synapse                           |
|--------------------|--------------------------------|-----------------------------------|
| Primary use-case   | Financial ledger (debit/credit)| AI agent memory / semantic search |
| Guarantees         | Strict-Serializable, replicated| Eventually-consistent WAL SQLite  |
| Data model         | Fixed: accounts + transfers    | Free: text + vec + graph + meta   |
| Query model        | Lookup by ID / filter          | BM25 + cosine + hybrid RRF        |
| Durability default | fsync always                   | synchronous=NORMAL (benched =OFF) |

---

## Verdict

| Scenario                              | Winner         |
|---------------------------------------|----------------|
| Financial ledger, double-entry accounting | TigerBeetle (purpose-built, 2× faster, guaranteed correct) |
| AI agent memory, RAG, semantic search | Synapse (TB has zero search capability) |
| Append-only event log + FTS           | Synapse        |
| Multi-model store (vec + text + graph)| Synapse        |
| Cluster replicated OLTP               | TigerBeetle    |
| Single-binary embedded, no server     | Synapse        |

**Synapse is ~2× slower on pure insert throughput (44k vs 88k tx/s)** when simulating TB's native workload with generic JSON docs. At 44k tx/s Synapse still handles most real-world agent memory workloads comfortably. TigerBeetle's strict guarantees + 2× throughput advantage make it the clear winner for financial/ledger use-cases — a domain Synapse never targets.
