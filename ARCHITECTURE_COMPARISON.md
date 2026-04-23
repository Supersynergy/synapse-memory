# Synapse Architecture: Single-File vs Multi-File

## Benchmark Context (M4 Max, aarch64, Criterion 200 samples)

All numbers measured on this hardware with our Phase 1 optimizations (NEON SIMD + int8 quantization).

## Current Architecture: Single-File + Turbo Overlay

```
synapsedb.db (SQLite)           turbo (in-memory)
+-----------+---------+       +------------------+
| docs      | FTS5    |       | NdArraySearch    | ndarray f32 matrix
| vec0      | vec_idx |       | QuantizedSearch  | int8 flat matrix
| crdt_log  | meta    |       | HybridCache      | 3-tier cache
+-----------+---------+       +------------------+
        |                              |
        +--- search(Vec) ----> sqlite-vec (65-913 us)
        +--- search(Fts) ----> FTS5 BM25 (<1ms)
                                       |
        turbo path -----> simd-f32 (2.6-113 us)
                     +--> quantized-i8 (1.8-66 us)  <-- fastest
```

## Approach A: Pure Single-File (Current + Optimized)

**Design**: One SQLite file per brain. Turbo overlays are ephemeral in-memory structures built on open.

### Strengths
| Property | Value |
|----------|-------|
| Atomic backup | `cp synapsedb.db synapsedb.db.bak` |
| CRDT merge | Single source of truth, yrs document per row |
| Ed25519 signatures | Inline per-row, verifiable offline |
| ACID transactions | SQLite WAL mode, one writer |
| Deploy | Single binary + single file |
| Encryption | SQLCipher bundled, single passphrase |

### Benchmark (5k docs, 384-dim, k=10)
| Strategy | Latency | Memory |
|----------|---------|--------|
| sqlite-vec (baseline) | 913 us | 0 (disk-backed) |
| ndarray-f32 | 147 us | 7.3 MB |
| simd-f32 | 113 us | 7.3 MB |
| quantized-i8 | 66.5 us | 1.9 MB |

### Scaling Limits
- sqlite-vec linear scan: O(n*d) per query
- At 50k docs: ~1.3ms (quantized), ~9ms (sqlite-vec) -- still sub-10ms
- At 500k docs: ~13ms (quantized), ~91ms (sqlite-vec) -- first pain point
- At 1M+: need IVF sharding (already implemented in `shard.rs`)

---

## Approach B: Multi-File (Separated Concerns)

**Design**: Separate storage engines per concern, coordinated by a manifest.

```
brain/
  manifest.toml          # shard registry + metadata
  docs.db                # SQLite: text + metadata + CRDT + signatures
  fts.db                 # SQLite FTS5 only (BM25)
  vectors.bin            # mmap'd flat f32/i8 matrix (no SQLite overhead)
  vectors.idx            # IVF centroids + posting lists
  cache.bin              # mmap'd result cache (optional)
```

### Strengths
| Property | Value |
|----------|-------|
| Vector I/O | mmap'd binary, no SQLite decode overhead |
| Parallel read | Separate file locks per concern |
| Hot/cold split | Vectors in RAM, docs on disk |
| Incremental backup | Only changed files need sync |

### Projected Benchmark (5k docs, 384-dim, k=10)
| Strategy | Est. Latency | Reasoning |
|----------|-------------|-----------|
| mmap f32 brute | ~90-100 us | No SQLite wrapper, direct memory |
| mmap i8 brute | ~50-55 us | Same + 4x smaller working set |
| IVF-quantized | ~5-15 us | Scan only ~3% of vectors (nprobe=3/32) |

### Complexity Costs
| Concern | Impact |
|---------|--------|
| Atomic backup | Need coordinated snapshot across files |
| CRDT merge | Must sync docs.db + vectors.bin atomically |
| Encryption | Each file needs separate encryption or envelope |
| Deploy | Binary + directory instead of binary + file |
| Corruption recovery | More failure modes (partial write to vectors.bin) |
| Code complexity | +500-800 LOC for manifest, mmap, coordination |

---

## Decision Matrix

| Criterion | Weight | Single-File | Multi-File | Notes |
|-----------|--------|-------------|------------|-------|
| Simplicity | 25% | **10** | 5 | Single file = single concern |
| Sub-50k perf | 20% | **9** | 10 | Both sub-ms; multi-file marginally faster |
| >500k perf | 15% | 5 | **9** | IVF + mmap wins at scale |
| Backup/sync | 15% | **10** | 4 | cp vs coordinated snapshot |
| CRDT merge | 10% | **10** | 6 | Atomic merge vs multi-file coordination |
| Encryption | 10% | **9** | 5 | SQLCipher vs per-file envelope |
| Memory efficiency | 5% | 7 | **9** | mmap = OS-managed paging |
| **Weighted Score** | | **8.65** | **6.35** | |

## Recommendation: Hybrid (A + selective B)

**Keep single-file as the default.** The current architecture with turbo overlays is the right approach for Synapse's target use case (<50k docs per brain, portable, atomic).

**Adopt multi-file only for the >50k scale path** via the existing `shard.rs` IVF system:

```
<50k docs:  synapsedb.db + in-memory turbo (quantized i8)
>50k docs:  shard.rs splits into N shard SQLite files + IVF index
>500k docs: Optional mmap vectors.bin per shard (Phase 3)
```

### What to build next (Phase 2-3 from masterplan):

1. **Cross-encoder reranking** (Phase 2): After quantized search retrieves top-50, rerank with a small cross-encoder model for precision. This works identically in both architectures.

2. **Matryoshka dimension truncation** (Phase 1c, not yet done): Store full 384-dim but search at 128-dim or 64-dim first, refine matches at full dim. Gives 3-6x throughput boost for free.

3. **IVF + quantized shards** (Phase 3): The existing `shard.rs` already splits by k-means centroids. Add quantized search per shard + bloom prefilter = sub-ms at 1M+ docs.

### Architecture evolution path:

```
TODAY:     SQLite single-file + int8 quantized turbo (66us/5k)
PHASE 2:   + Matryoshka 128-dim first pass (~22us/5k est.)
           + Cross-encoder rerank top-50
PHASE 3:   + IVF sharding with quantized sub-indexes
           + Optional mmap for hot shards
FUTURE:    Multi-file ONLY where needed (>500k per shard)
```

The single-file approach wins on every metric that matters for Synapse's design philosophy: portability, simplicity, atomic operations, and encryption. The performance gap is negligible at the target scale, and the turbo overlays close it to <1ms for up to 50k docs.
