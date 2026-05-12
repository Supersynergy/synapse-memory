# SOTA ANN Mining for Synapse — 2026-05-05

**Baseline (current Synapse stack)**: 168k×384 BGE-small, M4 Max NEON+SimSIMD.
- usearch M=48, ef_s=64: **1631 QPS @ R=0.982**
- binary cascade (Hamming → f32): **697 QPS @ R@10=0.994**
- target: push past usearch baseline at R≥0.99.

## Findings table

| # | tech | repo | language | maturity | last commit | expected_gain vs usearch M=48 | LOC_to_integrate | recommendation |
|---|------|------|----------|----------|-------------|------------------------------|------------------|----------------|
| 1 | **RaBitQ + IVF (lqhl/rabitq-rs)** | github.com/lqhl/rabitq-rs | Rust (crate `rabitq-rs` 0.9) | prod-ready x86; **ARM64 broken** | 2026-02-26 | 2-4× QPS @ R=0.99 (4-bit), 32× memory cut, FHT rotation, fastscan | ~150 LOC (Cargo dep + train + search wrapper) | **BLOCKED on M4 — ARM64 distance bugs documented**; track upstream |
| 2 | **VectorChord (tensorchord)** | github.com/tensorchord/VectorChord | Rust (PG ext, but core lib extractable) | production, 1667★, RaBitQ8 native, hierarchical kmeans | 2026-04-30 | 100M vec / 20 min build, 4-bit/8-bit native types, autonomous reranking | ~600 LOC (extract IVF+RaBitQ core from `vchordrq` crate, drop pgrx) | **TOP PICK** — Rust, ARM-clean, battle-tested at 1B scale |
| 3 | **kemingy/rabitq** | github.com/kemingy/rabitq | Rust | small (10★) but pure Rust, AGPL | 2026-04-23 | matches RaBitQ paper @ R=0.99 with SIMD | ~80 LOC | fallback if VChord extract fails; **AGPL license blocks commercial** |
| 4 | **Microsoft DiskANN (Rust rewrite)** | github.com/microsoft/DiskANN | **Rust** (main branch, 7.3MB code, 1783★) | active rewrite as of 2026-05-05 | 2026-05-05 | 2-3× QPS, single-pass build, single-graph fresh inserts (Fresh-DiskANN) | ~400 LOC | secondary — wait for v1.0 tag, currently unstable |
| 5 | **pgvectorscale StreamingDiskANN** | github.com/timescale/pgvectorscale | Rust+pgrx, 2996★ | production | 2026-04-30 | DiskANN + Statistical Binary Quantization, **28× lower p95 vs Pinecone** | ~500 LOC (extract `vectorscale` rust crate) | high value but PG-coupled; SBQ algorithm worth porting standalone |
| 6 | **VectorDB-NTU/RaBitQ-Library** | github.com/VectorDB-NTU/RaBitQ-Library | C++ (official) | gold reference, 202★, multi-bit RaBitQ | 2026-05-04 | reference impl, 4/5/7-bit → 90/95/99% recall *no rerank* | ~300 LOC (cxx FFI bridge) | reference for correctness; not direct ship |
| 7 | **ParlayANN (cmuparlay)** | github.com/cmuparlay/ParlayANN | C++ | research, 186★ | 2026-01-05 | **10× faster HNSW build** via parallelism, supports HCNNG/Vamana/HNSW | ~800 LOC FFI | build-time win only; query side already saturated by usearch |
| 8 | **antgroup/vsag (HGraph + RaBitQ)** | github.com/antgroup/vsag | C++, 470★ | prod (Ant Group) | 2026-04-30 | HGraph hybrid claims top of ann-benchmarks 2025 | ~600 LOC FFI | strong alt; C++ FFI overhead vs native Rust |
| 9 | **nnethercott/hannoy** | github.com/nnethercott/hannoy | Rust, 80★ | prod, LMDB-backed HNSW | 2026-04-08 | KV-backed HNSW, persistence + warm-start | ~100 LOC | useful for disk tier, not in-mem QPS |
| 10 | **meilisearch/arroy** | github.com/meilisearch/arroy | Rust, 302★ | prod | 2026-04-07 | Random projections + LMDB, low memory | ~100 LOC | not competitive at R≥0.99 |
| 11 | **Multi-bit RaBitQ (SIGMOD 2025)** | arxiv 2409.09913 | paper + C++ in NTU lib | published | — | optimal error bound, 4/5/7-bit ladder | shared with #1/#6 | algorithm, not code |
| 12 | **Matryoshka cascade (384→128→32)** | own impl on BGE-small | Rust | DIY | — | 3-5× QPS via dim-cascade if BGE-small supports MRL | ~120 LOC | only if embedder produces nested dims; BGE-small does *not* by default → re-embed needed |
| 13 | **easy_tiger / DiskANN-WT** | github.com/mccullocht/easy_tiger | Rust, 6★ | exp | 2026-04-08 | DiskANN on WiredTiger | ~250 LOC | research only |
| 14 | **glass (zilliztech/pyglass)** | github.com/zilliztech/pyglass | C++, 142★ | stale | 2025-09-09 | NSG+HNSW with SQ4U/SQ8U quant, claims ~2× faiss-hnsw | ~400 LOC FFI | **stale 8 months** — do not adopt |
| 15 | **SOAR (ScaNN successor)** | google-research/scann | C++ | published 2024, no standalone Rust | — | spilling routing, ~10-20% recall lift at fixed QPS | ~1k LOC (rewrite) | high effort, low ROI vs RaBitQ |

## Top 3 (ranked by ROI)

1. **VectorChord IVF+RaBitQ core extract** — Rust, ARM-clean, 1667★, RaBitQ8 native, hierarchical k-means proven to 1B. Best engineering substrate.
2. **kemingy/rabitq** — pure Rust, MIT-incompatible AGPL but viable as research bench. Quick spike.
3. **Microsoft DiskANN Rust** — strategic bet, monitor v1.0 tag (graph + Fresh inserts unique).

## Concrete Integration Plan — Winner: VectorChord IVF+RaBitQ

**Goal**: replace usearch M=48 with IVF+RaBitQ4 + f32 rerank cascade. Target **3000+ QPS @ R≥0.99**, 8× memory cut (168k×384×4 = 258MB → ~32MB codes + 258MB raw).

### Phase 0 — Spike (1 day)
- `cargo add rabitq-rs` (lqhl) on **x86 Linux box** (avoid M4 ARM bug). Bench against current SIFT-like 168k×384 dump.
- Measure: build time, R@10 vs ef-equivalent budget, QPS at 4-bit / 5-bit / 7-bit.
- Expected: 4-bit ≈ 90% R no-rerank → cascade to f32 top-50 → R@10≥0.99 @ 2-3× usearch QPS.

### Phase 1 — VectorChord core extraction (3-5 days)
- Clone `tensorchord/VectorChord`. Target crate: `crates/vchordrq` (IVF+RaBitQ logic, separable from pgrx).
- Strip `pgrx`/`pgvector` deps; keep: hierarchical k-means, RaBitQ4/RaBitQ8 quantizer, RNG residual rotation, SIMD scan, reranker.
- Wrap in `synapse-rabitq` crate exposing `train(vectors, nlist) → Index`, `search(q, k, nprobe) → Vec<(id, dist)>`.
- ARM64 SIMD: VectorChord uses `std::simd` portable + scalar fallback → works on M4. Verify with `cargo nextest`.

### Phase 2 — Cascade integration (2 days)
- New cascade: **Hamming pre-filter (existing) → RaBitQ4 IVF (new) → f32 rerank top-50**.
- Tune: `nlist = sqrt(N) ≈ 410`, `nprobe = 16-32`, rerank_k = 50.
- Wire into Synapse hybrid retrieval path; A/B vs current cascade on eval/ benchmark suite.

### Phase 3 — Bench + ship (1 day)
- `hyperfine` build time, `cargo bench` QPS, full ann-benchmarks-style recall sweep.
- Pass criteria: **≥2.5× QPS @ R@10≥0.99**, build ≤30s for 168k vectors, memory ≤80MB index footprint.
- If win: replace usearch path; keep usearch as fallback flag.

### Risks + mitigations
- **R**: VectorChord core not cleanly separable from pgrx → **M**: fall back to `kemingy/rabitq` (smaller, AGPL → use only as research/bench, not ship).
- **R**: ARM64 SIMD perf regression vs x86 → **M**: verify NEON path, profile with `cargo flamegraph`; SimSIMD already proven on M4.
- **R**: 168k corpus too small for IVF benefit (IVF shines >1M) → **M**: pre-bench Phase 0 with synthetic 1M to confirm scaling; for 168k, MSTG (rabitq-rs v0.9) may be better than IVF.

### Out-of-scope / explicit drops
- glass/pyglass (stale 8mo)
- ParlayANN (build-time only, no query gain)
- SOAR/ScaNN (no Rust path, high port cost)
- Matryoshka (requires embedder change, separate workstream)

## Provenance
GitHub API + raw README (2026-05-05): VectorChord, ms/DiskANN, lqhl/rabitq-rs, kemingy/rabitq, VectorDB-NTU/RaBitQ-Library, pgvectorscale, pyglass, ParlayANN, vsag, hannoy, arroy, easy_tiger. arxiv 2405.12497 (RaBitQ SIGMOD24), 2409.09913 (multi-bit RaBitQ SIGMOD25).
