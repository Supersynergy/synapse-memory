# synapse-ultra — Architecture Spec

**Status:** Draft v1.0 | **Date:** 2026-04-25 | **Owner:** Maxim Supersynergy

## 1. Mission & Non-Goals

**Mission:** Pure-Rust standalone daemon replacing `~/.claude/synapse-turbo.py` (port 9477) over `~/.synapse/brain.db` (162k × 384-dim f32). Targets: T0 ≤ 5µs, T1 p50 ≤ 2ms / 5k QPS/core, ≤ 250 MB resident, single static binary.

**Non-Goals:** No HNSW (brute-force SIMD wins at 162k), no GPU, no write path (synapsed owns writes), no async beyond axum (search path is sync + rayon), no serde for snapshot (raw LE mmap), no cross-process cache.

## 2. Crate Layout

```
crates/synapse-ultra/
├── Cargo.toml
├── src/
│   ├── lib.rs              # Public API: UltraIndex, Hit, Query
│   ├── main.rs             # bin: synapse-ultra
│   ├── config.rs           # clap + env
│   ├── index/
│   │   ├── mod.rs          # UltraIndex (ArcSwap holder)
│   │   ├── snapshot.rs     # mmap loader / atomic rebuilder
│   │   ├── matrix.rs       # f32 [N,D] + ID map
│   │   └── binary.rs       # 1-bit sketch (48B/doc @ 384d)
│   ├── resolver/
│   │   ├── mod.rs          # 4-tier orchestrator + auto-route
│   │   ├── t0_cache.rs     # quick_cache LRU + bloom neg-cache
│   │   ├── t1_simd.rs      # simsimd dot, NEON
│   │   ├── t2_binary.rs    # popcnt top-200 → f32 rerank
│   │   └── t3_sqlitevec.rs # rusqlite cold fallback
│   ├── wire/
│   │   ├── http.rs         # axum :9477
│   │   └── uds.rs          # /tmp/synapse-ultra.sock, len-prefixed msgpack
│   ├── metrics.rs          # atomic counters → /stats
│   └── errors.rs
├── benches/{tier0,tier1,tier2,e2e_qps}.rs   # criterion
├── bench/run.sh
├── tests/{recall_regression,snapshot_rebuild,http_smoke}.rs
└── README.md
```

Separate binary from `synapsed`. Re-exports `synapse-core::turbo::ndarray_search` for T1 to avoid duplication.

## 3. Storage — `~/.synapse/ultra_matrix.bin`

Raw LE layout:
```
MAGIC u64 (0x53594E55_4C545241 "SYNULTRA")
VERSION u32 = 1
DIM u32 = 384
N u64
BRAIN_MTIME_NS u64                 # staleness key
BRAIN_DOC_COUNT u64                # sanity
ID_TABLE [i64; N]                  # row → doc_id
MATRIX [[f32; DIM]; N]             # L2-normalized, row-major
BINARY_SKETCH [[u8; DIM/8]; N]     # sign-bit, 48B/doc
CRC32 u32                          # over matrix bytes
```

162k × (1536 + 48) ≈ **257 MB on disk, ~250 MB resident** via `memmap2`.

**Lazy rebuild:** stat brain.db.mtime vs header → if stale/missing/bad-CRC → rebuild from `docs_vec` (rusqlite read-only, `PRAGMA query_only=1`), write `.tmp`, fsync, atomic rename. Target rebuild: ≤ 3s for 162k via rayon (read + L2-norm + sign-pack).

**Hot reload:** `SIGHUP` or `POST /reload` → rebuild check → `ArcSwap::store(Arc::new(new))`. Old readers drain naturally.

## 4. Four-Tier Resolver

```rust
pub enum Tier { T0Cache, T1Simd, T2Binary, T3Sqlite }
pub struct Hit { pub doc_id: i64, pub score: f32, pub tier: Tier }
pub enum Mode { Auto, ForceTier(Tier) }

pub struct Query<'a> {
    pub vec: &'a [f32],
    pub k: usize,
    pub min_score: Option<f32>,
    pub filter: Option<&'a BitVec>,   // doc_id mask, predicate pushdown
    pub mode: Mode,
}
```

### T0 — Query LRU
```rust
pub struct T0Cache {
    lru: quick_cache::sync::Cache<u128, Arc<Vec<Hit>>>,
    neg: bloomfilter::Bloom<u128>,
    hits: AtomicU64,
    misses: AtomicU64,
}
```
Key = `blake3(vec_bytes ‖ k.to_le_bytes() ‖ filter_xxh)` truncated to u128. Capacity 32k entries (~6 MB). **Target: ≤ 5 µs lookup, 200k QPS/core.**

### T1 — simsimd SIMD dot
```rust
pub struct T1Simd<'a> { matrix: &'a Array2<f32>, ids: &'a [i64] }

impl T1Simd<'_> {
    pub fn search(&self, q: &[f32], k: usize, filter: Option<&BitVec>) -> Vec<Hit>
    // Path A (Apple/NEON): simsimd::f32::dot per row tight loop
    // Path B (x86 fallback): ndarray::Array2::dot(query) via matrixmultiply
    // top-k: BinaryHeap<Reverse<OrdF32>> size k; rayon over 4096-row chunks
}
```
Pre-normalized matrix → cosine == dot, no sqrt hot path. **Target: 2 ms p50, 5k QPS/core, 30k QPS @ 8c.**

### T2 — Binary rerank
```rust
pub struct T2Binary<'a> { sketch: &'a [u8], matrix: &'a Array2<f32>, ids: &'a [i64] }
```
Sign-pack query → 48 B. Hamming via `u64::count_ones` × 6 per doc → ~12 ns/doc → 162k in ~2 ms. Top-200 → f32 rerank → top-k. **Target: 0.5 ms p50, 20k QPS, recall@10 ≥ 95%.** Used when `Auto && k≤50 && N≥50k`.

### T3 — sqlite-vec fallback
`Mutex<Connection>`. Cold path only (snapshot missing, complex SQL filter, parity test). If hot → bug.

### Auto-route
```
T0.hit → return
filter && popcount<N/4 → T1+mask
k≤50 && N≥50k → T2
else → T1
on error → T3
always → T0.put
```

## 5. Concurrency

```rust
pub struct UltraDaemon {
    index: ArcSwap<UltraIndex>,
    cache: Arc<T0Cache>,
    sql_pool: Arc<T3Sqlite>,
    metrics: Arc<Metrics>,
}
```

Reads: `index.load()` lock-free ~10 ns. Writes: build off-thread, store. Tokio workers = 4; rayon pool = `num_cpus-2`. axum handlers wrap T1/T2 in `spawn_blocking`. **Two-level: tokio = request, rayon = data. Don't cross.**

## 6. Wire Protocols

### HTTP axum :9477 (drop-in for Python turbo)
| Route | Method | Body | Returns |
|-------|--------|------|---------|
| `/ping` | GET | — | `{ok,version,uptime_s}` |
| `/stats` | GET | — | tier counts, p50/p95, mem, snapshot mtime |
| `/vec` | POST | `{vector:[..384],k,filter_ids?}` | `{hits:[{doc_id,score,tier}],ms}` |
| `/find` | POST | `{text,k}` | FTS5 via brain.db |
| `/hybrid` | POST | `{text,vector,k,alpha}` | RRF/weighted merge |
| `/reload` | POST | — | force rebuild check |

JSON only. Accept `Content-Encoding: zstd` for big requests.

### UDS `/tmp/synapse-ultra.sock` (length-prefixed msgpack)
```
[u32 LE payload_len][rmp-serde Request]
[u32 LE payload_len][rmp-serde Response]
```
```rust
#[derive(Serialize, Deserialize)]
#[serde(tag="op")]
pub enum UdsRequest {
    Vec { vector: Vec<f32>, k: usize, filter: Option<Vec<i64>> },
    Hybrid { text: String, vector: Vec<f32>, k: usize, alpha: f32 },
    Stats, Ping,
}
```
Saves 40-80 µs vs HTTP. Used by `syn` CLI.

## 7. Throughput Targets (M4 Max)

| Tier | p50 | p95 | QPS/1c | QPS/8c | Mem |
|------|-----|-----|--------|--------|-----|
| T0 hit | 5 µs | 12 µs | 200k | 1.5M | 6 MB |
| T1 simsimd | 2 ms | 4 ms | 5k | 30k | 250 MB |
| T2 binary | 0.5 ms | 1.2 ms | 20k | 120k | +8 MB |
| T3 sqlite | 50 ms | 90 ms | 200 | 200 | shared |
| HTTP overhead | +80 µs | +200 µs | — | — | — |
| UDS overhead | +20 µs | +60 µs | — | — | — |

**vs Python turbo:** cache-hit 120× faster, T1 1.5× faster (numpy BLAS ≈ simsimd NEON), memory 2.4× lighter, cold-start 30× faster (mmap vs numpy load).

## 8. Cargo.toml

```toml
[dependencies]
synapse-core = { path = "../synapse-core" }
simsimd      = "6.5"
ndarray      = { version = "0.16", features = ["matrixmultiply-threading"] }
arc-swap     = "1.7"
quick_cache  = "0.6"
bloomfilter  = "3"
axum         = { version = "0.7", features = ["macros"] }
tokio        = { version = "1", features = ["full"] }
tower        = "0.5"
rmp-serde    = "1.3"
serde        = { version = "1", features = ["derive"] }
serde_json   = "1"
blake3       = "1.5"
memmap2      = "0.9"
rusqlite     = { version = "0.32", features = ["bundled"] }
sqlite-vec   = "0.1"
half         = { version = "2", optional = true }
bitvec       = "1"
rayon        = "1.10"
clap         = { version = "4", features = ["derive","env"] }
tracing      = "0.1"
tracing-subscriber = "0.3"
crc32fast    = "1.4"
zstd         = "0.13"

[dev-dependencies]
criterion    = { version = "0.5", features = ["html_reports"] }
proptest     = "1"
reqwest      = { version = "0.12", features = ["json"] }

[features]
default = []
f16 = ["half"]
```
**Pure Rust** (rusqlite bundled SQLite C amalgamation only acceptable concession).

## 9. Bench Harness `bench/run.sh`

```bash
#!/usr/bin/env bash
set -euo pipefail
QUERY_SET=~/projects/synapse/eval/queries_1k.json
for stack in sqlitevec pythonturbo coreNdarray ultraHttp ultraUds; do
  hyperfine --warmup 50 --runs 1000 \
    "scripts/bench_client.py --stack $stack --queries $QUERY_SET --k 10" \
    --export-json out/$stack.json
done
python3 scripts/recall_diff.py --baseline sqlitevec --target ultraHttp --target ultraUds
python3 scripts/report.py out/*.json > docs/synapse-ultra-bench-$(date +%F).md
```

**Pass criteria:** ultraUds p50 ≤ pythonturbo×0.5; ultraHttp p50 ≤ pythonturbo×1.0; recall@10 ≥ 0.99 vs sqlitevec; T2 ≥ 0.95.

## 10. Testing

- **Unit:** LRU eviction, bloom FP <1%; snapshot round-trip + bad-CRC → rebuild; T1 vs scalar dot within 1e-5.
- **Integration:** `recall_regression.rs` (100 golden queries, ≥0.99 across commits); `snapshot_rebuild.rs` (touch brain.db → rebuild ≤ 3s); `http_smoke.rs`.
- **criterion** µbench per tier. **proptest** for random vectors / k / filters — no panic / no NaN.
- **Load:** `wrk -t8 -c100 -d30s` → ≥ 30k QPS aggregate, p99 ≤ 10 ms.

## 11. Migration Plan

- **Week 1 — Shadow:** ultra on `:9478`. `syn-hybrid` dual-calls, logs diffs.
- **Week 2 — Swap:** launchd `com.supersynergy.synapse-ultra.plist` binds `:9477`. Python plist disabled (kept 30d for rollback).
- **Week 3 — UDS:** `syn` CLI prefers `/tmp/synapse-ultra.sock`, HTTP fallback. `synapse-core::UltraClient` auto-detects.
- **Week 5 — Decommission:** delete `synapse-turbo.py` after 14d zero-error. Update `~/.claude/CLAUDE.md`. `syn put --title "synapse-turbo deprecated"`.
- **Rollback:** re-enable Python plist; ports separable. Snapshot is rebuildable artifact.

## 12. Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| simsimd NEON Apple-only | High | Med | Path B `ndarray::dot` fallback (pure Rust x86/ARM); CI matrix macos-arm + linux-x64 |
| Snapshot torn on power loss | Low | Med | Atomic rename + CRC32 → forced rebuild |
| brain.db schema drift | Med | High | Header stores expected row count; bump `SNAPSHOT_VERSION` on schema change |
| N grows 10× → mem blow-up | Med | High | f16 feature flag halves mem; document N≤500k support range; >500k → re-eval HNSW |
| rayon/tokio thread starvation | Low | Med | spawn_blocking isolation; rayon = `num_cpus-2`, tokio worker = 4 |
| simsimd 6.5 API churn | Med | Low | Pin exact version; abstract behind internal `dot()` trait |
| HTTP JSON @ 30k QPS | Med | Low | UDS+msgpack bypass; HTTP for compat not throughput |

## 13. Anti-patterns (DO NOT)

- ❌ HNSW/IVF/PQ at 162k. Re-eval at N ≥ 1M.
- ❌ `Vec<Vec<f32>>` matrix. Always `Array2<f32>` row-major.
- ❌ `serde_json` in hot path beyond HTTP boundary.
- ❌ Share `Connection` across threads; T3 mutex is intentional.
- ❌ Recompute L2-norm per query; document client contract.
- ❌ Add write API; ultra is read-replica.
- ❌ `tokio::sync::RwLock` for index. ArcSwap is the right primitive.
- ❌ Per-request INFO logs; sample 1/1000 + atomic counters.
- ❌ Ship without `recall_regression.rs` green.

## 14. Open Questions (post-v1)

1. f16 quantization (~125 MB matrix, 2× cache, recall TBD — bench first).
2. Matryoshka shortcut (first 64 dims coarse → 384 rerank). Only if T2 < 95%.
3. Multi-tenant filter cache. Probably YAGNI.
4. gRPC. Only if a client demands over UDS+msgpack.
5. HTTP/2 + zstd response. Demand-driven.

## 15. Acceptance Checklist

- [ ] Tests green on macos-arm64 + linux-x86_64
- [ ] `bench/run.sh` meets §7 targets
- [ ] Recall@10 ≥ 0.99 vs sqlite-vec on 1000-q set
- [ ] launchd plist + man page + `--help`
- [ ] ≤ 250 MB resident @ sustained 10k QPS
- [ ] Cold-start ≤ 3s (valid snapshot) / ≤ 5s (rebuild)
- [ ] `syn-hybrid` shadow parity ≥ 7d, score-delta ≤ 0.001
- [ ] README + spec + bench report committed
