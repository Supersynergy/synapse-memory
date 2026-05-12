# Synapse-X Next Push — 16.6× → 100×+ HONEST gates

Date: 2026-05-12. Source survey: `SYNAPSE-X-SOTA-2026.md`.

Current stacked gate: **16.6×**. Target: **100×+ honest** across recall, latency, ingest, memory.

The 5 changes below are ordered by expected multiplier × confidence × ship-ease. Math is stacked multiplicative (each layer hits a different bottleneck).

---

## PUSH 1 — RaBitQ Index (1-bit + multi-bit refinement) [3–10×]

**Status**: FAISS-mainline (`facebookresearch/faiss/IndexIVFRaBitQFastScan.cpp`). Rust port live (`lqhl/rabitq-rs` — RaBitQ + IVF + **MSTG** multi-scale tree graph).

**What changes**: Replace flat-Hamming S4 path with **RaBitQ + IVF + MSTG re-ranker**. Memory drops 8–32×, recall stays >0.95, latency improves because re-rank-set is 10–50× smaller.

**Expected**: 3–10× on **memory-normalized recall@latency**. Honest, not a kernel trick — the algorithm strictly dominates flat-1-bit at scale.

**Risk**: low. Mature algo, in FAISS, in Rust. Drop-in.

**Effort**: 1–2 weeks port `lqhl/rabitq-rs` API into Synapse `multi_index` layer.

---

## PUSH 2 — Lance v3 Encoding for Columnar Tick Layer [2–5×]

**Status**: VERIFIED — `lance-format/lance@main rust/lance-encoding` exposes per-column metadata: `compression`, `compression-level`, `rle-threshold`, `minichunk-size`, `dict-divisor`, `structural-encoding`.

**What changes**: Synapse-X tick/doc storage moves from raw mmap+SQLite-vec to **Lance-encoding mini-chunks** with adaptive RLE+bit-pack+dict per column. M4 L1-line-aligned mini-chunks. Random-access stays O(1).

**Expected**: 2–5× on **scan throughput** and 3–8× on **disk footprint** vs Parquet baseline (Lance docs claim 100× for ML random-access but we cite honest scan-only).

**Risk**: medium. Need to fork or wrap `lance-encoding` crate cleanly. Bench harness `Xuanwo/lance-dataset-test` already exists.

**Effort**: 2–3 weeks.

---

## PUSH 3 — Accelerate.framework (AMX-backed) Matrix Route [2–8× for large dot products / matmul]

**Status**: VERIFIED — Apple's Accelerate silently routes through AMX for matrix sizes above ~64. MPSGraph callable from Rust via Objective-C bridge (`progrium/darwinkit` proves it works from Go; same pattern in Rust via `objc2`).

**What changes**: Add a routing layer: dot/cosine on vectors of dim ≤256 → SimSIMD (current king); batch-cosine over N≥64 queries → **Accelerate `cblas_sgemm`** (AMX); large kernels → MPSGraph.

**Expected**: **2–8× batched cosine** (which is the realistic ANN load — you score 100s of candidates per query). No work for AMX; Apple does it.

**Risk**: low. Standard pattern. AMX is internal — we never touch its opcodes.

**Effort**: 3–5 days for `vDSP_mmul` + `cblas_sgemm` wrapper; 1 week routing logic.

---

## PUSH 4 — Adaptive Plan-Cache (mmap-persisted, bandit-routed) [1.5–5×]

**Status**: VERIFIED no embedded competitor. DataFusion has no plan-cache (ghgrep 0 hits). DuckDB has prepared-statement cache only.

**What changes**: Persist per-query-shape operator-plan in a Synapse-X mmap-page. Thompson-bandit chooses among (a) HNSW-first, (b) IVF-first, (c) RaBitQ-1bit-then-multi, (d) flat-scan, based on observed per-shape latency. Decay weights weekly.

**Expected**: 1.5–5× on **mixed workloads** where current static routing picks wrong. Pure win on heterogeneous query mix (the realistic case).

**Risk**: medium. New code path. But aligns with existing AdaptiveRouter Thompson infrastructure.

**Effort**: 1–2 weeks integrate into existing AdaptiveRouter.

---

## PUSH 5 — Vortex Cascading-Encoding for Cold-Tier + Online Learner mmap-state [1.5–3× + free moat]

**Status**: VERIFIED Vortex in active dev + adopted by Apache Paimon, Milvus. Online-learner-in-mmap: NO competitor found (Target 6 + Target 8 gap).

**What changes**:
(a) Cold-tier of Synapse-X uses Vortex cascading-encoding (write-once, scan-fast) — complements Lance-encoding for hot tier.
(b) Add online linear/logistic learner with weights+momentum living in mmap-page (hash-trick keyed, à la Vowpal). Lets Synapse-X *learn its own router/recall-tuner* from query logs without an external trainer.

**Expected**: 1.5–3× cold-tier scan + new feature class (compounding moat: nothing else does this embedded).

**Risk**: medium. Vortex API still evolving. Online learner is greenfield code, but small (~500 LoC Rust).

**Effort**: 2 weeks Vortex hot-path; 1 week online learner.

---

## Stacked-multiplier math

Conservative (low end, all push):

| Layer | Multiplier |
|---|---|
| Current baseline | 16.6× |
| PUSH 1 (RaBitQ) | × 3.0 |
| PUSH 2 (Lance v3) | × 2.0 |
| PUSH 3 (Accelerate/AMX route) | × 2.0 |
| PUSH 4 (Plan-cache) | × 1.5 |
| PUSH 5 (Vortex cold + online) | × 1.5 |
| **Stacked total** | **16.6 × 3 × 2 × 2 × 1.5 × 1.5 = 448×** |

Honest discount (gates rarely stack 100% — interactions, regressions, reality): apply **0.25 discount** → **~110×**. Above the 100× target with margin.

Aggressive (high end): 16.6 × 10 × 5 × 8 × 5 × 3 = ~100,000× (paper-only). Reject.

**Realistic plan: ship PUSH 1+3 first** (lowest risk, ~3× × 2× = 6× on top of 16.6 = **~100× honest** alone). Then 2, 4, 5 for compounding moat.

---

## Ship sequence (8-week plan)

- **Week 1–2**: PUSH 3 (Accelerate route) — fastest, lowest risk, unblocks bench
- **Week 2–4**: PUSH 1 (RaBitQ) — biggest single-layer win
- **Week 4–6**: PUSH 2 (Lance v3) — storage substrate
- **Week 6–7**: PUSH 4 (plan-cache) — wraps everything
- **Week 7–8**: PUSH 5 (Vortex + online learner) — moat

Bench infra (run continuously from week 1): **nextest + divan + CodSpeed** (Target 10).

---

## Out of scope (rejected)

- Direct AMX opcodes — fragile, undocumented, Apple may change.
- River-rs / Rust online-ML port — too speculative; ship our own minimal learner.
- WASM-component distribution — defer until v1 stable.
- DataFusion adoption — too heavy for embedded; we cherry-pick its planner ideas only.
- Tectonic / Solace / LeanDB — not OSS-mature in 2026.
- Photon / Velox — proprietary or wrong language.
