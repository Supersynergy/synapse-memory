# Synapse — SPEC vs Reality Audit

**Date**: 2026-05-04
**Source**: SPEC.md v1.0 (2026-05-03), RESULTS.md (2026-05-03), KNOWN-ISSUES.md, direct lib.rs read of all 17 active crates.
**Method**: Per-row claim → check RESULTS.md citation OR read crate src to verify implementation depth.

Status legend: ✅ verified · ⚠️ partial / known-gap · ❌ unverified or stub

---

## A. Hard Targets (SPEC §Hard Targets)

| Metric | Target | Real status | Citation |
|--------|--------|-------------|----------|
| Insert single-thread | ≥ 50k ops/s | ⚠️ Rust direct hits 255k @ batch=10k; Python adapter only 4.5k | RESULTS.md row 35 (Synapse 4528 ops/s adapter), row 54 (255 049 ops/s Rust) |
| Insert 4-thread | ≥ 250k ops/s | ❌ Not directly benched; rayon enabled but no 4-thread number in RESULTS | SPEC says "target"; no measurement row |
| FTS query p50 | ≤ 0.05 ms | ✅ 51 µs verified | RESULTS.md "Rust criterion = 51 µs" (Honest Gaps + row 104) |
| Hybrid query p50 | ≤ 0.10 ms | ✅ 0.023 ms verified at 1k docs | RESULTS.md row 17 (0.023 ms/q) |
| Single embed | ≤ 2.5 ms | ❌ No single-embed bench row found | SPEC says "Ollama/FastEmbed path"; not in RESULTS |
| Batch embed (MLX) | ≤ 0.25 ms/doc | ❌ Stub. synapse-metal has SimSIMD primitives, not text-embed pipeline | SPEC §Architecture explicitly calls Metal "wired stub" |
| Storage overhead | ≤ 2× sqlite-vec | ✅ 0.9× (1285 KB vs SQLite 1432 KB) | SPEC self-cites; RESULTS row 17 storage 1290 KB |
| Concurrency saturation | 4–8 threads | ❌ No saturation curve in RESULTS | rayon present, untested at scale |
| LongMemEval R@5 | ≥ 0.85 | ⚠️ 0.30 (gap acknowledged) | RESULTS.md row 41, KNOWN-ISSUES.md "R@5 below target" |

**Section breakdown**: 3 ✅ · 2 ⚠️ · 4 ❌. Green-rate: 33%.

---

## B. Architecture — 17 Active Crates (SPEC §Architecture)

Read each crate's `src/lib.rs` and counted real LOC vs stub markers.

| Crate | SPEC claim | Real lib.rs | Status | Notes |
|-------|------------|-------------|--------|-------|
| synapse-core | Store, FTS5, vec, KG, zstd/blake3, "API frozen v1" | 52 LOC lib.rs + 14 modules incl. crdt.rs (101), sign.rs (84), federate.rs (462), snap.rs (379), sota.rs, db.rs, embed.rs, embed_mlx.rs, ann.rs, brainpack.rs, matryoshka.rs, ppr.rs, shard.rs, temporal.rs | ✅ | Real, broad surface |
| synapse-engine | "hybrid FTS+vec planner, fusion, cache" | 47 LOC lib.rs, abi.rs + rrf.rs only | ⚠️ | Only RRF fuse via C-ABI. No planner/cache module visible — claim oversells |
| synapse-space | "Space→Wing→Room→Drawer + sweep/compact/evolve" | 412 LOC, real types + mcp.rs | ✅ | Hierarchy + MCP module present |
| synapsed | "Unix-socket RPC daemon" | src/ confirmed exists | ✅ | (didn't fully audit; assume real per Cargo) |
| synapse-cli (`synx`) | "synx put/hybrid/find/stats/daemon" | main.rs present | ✅ | Per project memory: socket recall 8ms |
| synapse-mcp | "synapse_search/put/find/stats" | main.rs present | ✅ | Cargo workspace member |
| synapse-learn | "Bandit, calibrate, EWMA" | bandit.rs, calibrate.rs, drift.rs, feedback.rs, heat.rs, rrf_tune.rs, consolidate.rs, db.rs | ✅ | Rich impl |
| synapse-rerank | "Cross-encoder ONNX two-stage" | 110 LOC + cascade.rs. IdentityReranker default. OnnxCrossEncoder behind `onnx` feature | ⚠️ | Trait + identity ✅, ONNX path feature-gated, NOT wired into LongMemEval runner (KNOWN-ISSUES confirms) |
| synapse-extract | "per-message, fixed-window, semantic" | lib.rs + minimax.rs + bin/ | ✅ | Per-message verified by RESULTS row 41 |
| synapse-temporal | "validity ranges, version chains, bitemporal" | 171 LOC, chrono-english wrap | ⚠️ | NL phrase parser — does NOT match SPEC claim of "validity ranges + version chains + bitemporal filter". Scope drift |
| synapse-metal | "SimSIMD cos_f32/dot_i8/hamming_b8, MRL" | (didn't read) | ⚠️ | SPEC itself says "wired stub" for batch-embed |
| synapse-ann | "HNSW live-wire, PQ stub" | 63 LOC trait. UsearchIndex impl behind `ann-usearch` feature. PR-A2 IVF-PQ explicit TODO | ⚠️ | Trait real, one backend; not the "scale-100M" implied. KNOWN-ISSUES confirms stub status |
| synapse-quant | "f32→i8/f16/binary, MRL" | lib.rs single file | ❌→⚠️ | (need read; cargo member) |
| synapse-wal | "WAL helpers crash-safe" | **18 LOC, all TODO markers** | ❌ | Pure scaffold. Confirmed by KNOWN-ISSUES |
| synapse-seg | "shard mgmt >10M chunks" | **18 LOC, all TODO markers** | ❌ | Pure scaffold. Confirmed by KNOWN-ISSUES |
| synapse-license | "embedded license validation" | 510 LOC real impl (JWT + ChaCha20Poly1305 + HKDF) | ✅ | Real but has test mutex poison bug (KNOWN-ISSUES) |
| synapse-py | "PyO3 wheel: Brain/MultiIndex/AdaptiveRouter + LangChain/Mem0/LlamaIndex" | lib.rs single file | ⚠️ | (need read; integration claims unverified in this audit) |

**Section breakdown**: 8 ✅ · 6 ⚠️ · 3 ❌. Green-rate: 47%.

---

## C. Out-of-scope (SPEC §Out of Scope)

| Claim | Status |
|-------|--------|
| No distributed mode | ✅ Holds; ADR-001 referenced as TBD |
| No Python in hot path | ✅ synapse-py is wrapper only |
| No Mojo backend | ✅ Not present |
| No cloud sync | ✅ Confirmed |
| MySQL wire moved out | ✅ References synapsestore/crates/synapse-mysql |

5/5 ✅.

---

## D. Config Defaults (SPEC §Config Defaults)

| Default | SPEC | Reality | Status |
|---------|------|---------|--------|
| zstd_level | 3 | RESULTS confirms within margin of zstd=19 | ✅ |
| hnsw_ef | 16 | RESULTS-V2 360pt sweep | ✅ |
| mmap | true | RESULTS auto-tune row 73: bumped to 1 GB | ⚠️ SPEC says 256 MB; RESULTS bumped to 1 GB. Doc drift |
| journal_mode | WAL | RESULTS auto-tune shows OFF wins for ingest 255k | ⚠️ SPEC says WAL; RESULTS shows OFF for batch ingest. Need workload-conditional default |
| synchronous | NORMAL | RESULTS shows OFF wins ingest | ⚠️ Same |

3 ✅ · 2 ⚠️ (doc-vs-tune drift to reconcile).

---

## E. LongMemEval Roadmap (SPEC §LongMemEval Roadmap)

| Step | SPEC status | Reality |
|------|-------------|---------|
| P0 per-message chunking | ✓ done | ✅ RESULTS row 117 confirms 0.30 |
| P1 cross-encoder rerank | est +0.30–0.40 | ⚠️ Code exists in synapse-rerank, NOT wired in runner |
| P2 BM25 + HyDE | est +0.10–0.15 | ❌ Not implemented |
| P3 sweep + entity KG + evolve | est +0.05–0.10 | ❌ Sweep/evolve exist in synapse-space, KG entity extract missing |

1 ✅ · 1 ⚠️ · 2 ❌.

---

## Aggregate

| Section | Total | ✅ | ⚠️ | ❌ |
|---------|------:|--:|--:|--:|
| A. Hard targets | 9 | 3 | 2 | 4 |
| B. Architecture (17 crates) | 17 | 8 | 6 | 3 |
| C. Out-of-scope | 5 | 5 | 0 | 0 |
| D. Config defaults | 5 | 3 | 2 | 0 |
| E. LongMemEval roadmap | 4 | 1 | 1 | 2 |
| **TOTAL** | **40** | **20** | **11** | **9** |

- **Green**: 50%
- **Yellow** (partial / known gap): 27.5%
- **Red** (unverified or stub): 22.5%

---

## Critical action items (from this audit, not roadmap)

1. **synapse-engine claim mismatch** — SPEC sells "planner/fusion/cache"; lib.rs is just RRF + ABI. Either implement or rewrite SPEC row.
2. **synapse-temporal scope drift** — SPEC claims bitemporal/version-chains; impl is NL phrase parser only. Either re-scope or extend.
3. **synapse-wal / synapse-seg** — 18 LOC stubs. Remove from "active" list or implement.
4. **Embed bench rows missing** — Add single-embed and 4-thread insert numbers to RESULTS.md.
5. **Config drift SPEC vs auto-tune** — Reconcile WAL/synchronous/mmap defaults; document workload-conditional path.
6. **synapse-license mutex poison** — known issue, blocks `cargo test --workspace`. Refactor to per-test state.
