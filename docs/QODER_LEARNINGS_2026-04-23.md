# Qoder Logs — Synapse-relevant Learnings, 2026-04-23

## Scope of scan

- `~/.qoder/logs/qodercli.log` — 4.1 MB / 48 200 lines, **0 hits** for `synapse|brain.db`
- `~/.qoder/specs/` — **1 hit**: `synapse-turbo-masterplan.md` (404 lines)
- `~/.qoderwork/projects/` — **6 hits** in workspace `mo8tmirivjhzyt3v`, project ID `78b9cdb9-31c1-474b-8e97-e730e9e61c73`. One `.jsonl` (421 lines, conversation transcript) plus 5 JSON snapshots.
- `~/Library/HTTPStorages/com.qoder.{ide,work}/httpstorages.sqlite` — present but unread (browser-storage cookies, not session content).
- `~/.qoder/extensions/...common_pip_packages.json` — incidental match (lists `synapse-rdkit` chemistry pkg, not us).

## Honest finding

Qoder authored a **comprehensive technical spec** for "Synapse Turbo" prior to this session. The masterplan reads like an engineering decision document — concrete files, line numbers, expected speedups, success criteria. It is **not** a passive transcript; it is a directive document. Below are the load-bearing decisions worth preserving.

## Decisions extracted from `~/.qoder/specs/synapse-turbo-masterplan.md`

### Architecture choices

1. **Tiered index by N**:
   - Tier 1 hot cache (`turbo HybridCache`)
   - Tier 2 in-memory (NdArraySearch + RaBitQ for 1-10 M)
   - Tier 3 disk-resident (DiskANN-style Vamana for >10 M)
   - Tier 4 distributed sharding (multi-node, optional)

2. **NEON intrinsics** for the hot dot-product loop (target 3-6× speedup over ndarray matmul, citing RuVector ADR-003).

3. **Scalar quantization int8** (target: 4× memory reduction, 2-3× search speedup, recall ≥ 0.95 vs f32).

4. **Matryoshka 384→256→128→64d** dimension truncation (target: 256d retains 97.5 % recall, −33 % storage, ~1.5× faster search).

5. **RaBitQ 1-bit quantization** for >1 M scale (recall ≥ 0.90 target).

6. **DiskANN/Vamana** for >10 M, ~10 % RAM vs HNSW's 100 %, sub-30 ms p95 for 1 B+.

7. **Lance sidecar** for >500 k (1.5 M IOPS per Lance v2 bench cited).

### Acceptance criteria (lifted directly)

| Metric | Current | Phase 1 Target | Phase 3 Target |
|---|---|---|---|
| Search latency 10 k | 0.03 ms | 0.01 ms (NEON) | 0.01 ms |
| Search latency 100 k | ~0.3 ms | 0.1 ms (int8) | 0.1 ms |
| Search latency 1 M | N/A | N/A | < 5 ms (HNSW + RaBitQ) |
| Memory / 10 k vecs | ~60 MB | ~15 MB (int8) | ~2 MB (RaBitQ) |
| Recall@10 | 1.0 brute | ≥ 0.95 (int8) | ≥ 0.90 (RaBitQ) |
| NDCG@10 hybrid | baseline | +6-9 (LTR) | +15 (cascade) |

### New feature flags Qoder proposed

`ltr` · `rerank` · `rabitq` · `lance` — none yet implemented in our scale-100M tree. Adopting these names verbatim if shipped, to keep the two roadmaps aligned.

## Cross-check vs this session's plan

| Qoder decision | This session's plan ref | Status |
|---|---|---|
| Tiered index by N | `SCALE_100M_PLAN_2026-04-23.md` §3 (PR-A1 usearch + PR-A2 IVF-PQ + PR-B1 fjall LSM) | **aligned** |
| NEON intrinsics | SPEC §6 item not yet ranked | gap — add as PR-C-fast (1 d) |
| int8 quant | Plan PR-C1 | **aligned** |
| Matryoshka 256d | Plan PR-C2 | **aligned** |
| RaBitQ 1-bit | not in plan | gap — add as PR-C3 candidate |
| DiskANN | not in plan (deferred to 100 M arithmetic) | **deferred deliberately** |
| Lance sidecar | not in plan (Lance has Apache Arrow dep weight) | **deliberately rejected** in favor of fjall (pure Rust LSM) per plan §2.2 |
| LTR rerank | plan PR-A2 sibling track | gap — add as PR-G-rerank |

## Items Qoder mentions that we should adopt

- **NEON intrinsics module** (`crates/synapse-core/src/turbo/simd.rs`) — concrete name, plan add as PR-C-fast.
- **`crates/synapse-core/src/turbo/quantize.rs`** — same concrete name as Qoder.
- **`crates/synapse-core/benches/turbo_bench.rs`** — comprehensive criterion harness across all turbo strategies.
- **Tests for federation/shard/encryption** — Qoder lists missing test files; track as TEST-DEBT.
- **Memory profiling per strategy** — track RSS at 10 k / 100 k / 1 M per algorithm. Currently we only measure RSS delta of the whole bench process.

## Items Qoder mentions that we should NOT adopt

- **Lance sidecar**: pure-Rust default-build constraint (SPEC §3) breaks if Lance pulls Arrow C++ stack.
- **`linfa-trees` LTR**: heavyweight dep; prefer ONNX-loaded XGBoost/LightGBM model behind feature flag.

## What was NOT in any Qoder log

- **No conversational decision log** referencing measured Synapse numbers (Qoder's spec uses target numbers, not measurements).
- **No competitive benchmark** vs Chroma / sqlite-vec / LanceDB — Qoder cites them but does not measure.
- **No PR-A1-wire equivalent** — Qoder names the tiered architecture but does not show a working integration.

## Action items into the Synapse roadmap

1. Adopt Qoder's file names and feature flags in plan §3.
2. Add **PR-C-fast (NEON intrinsics)** as a 1-day item between PR-C1 and PR-C2.
3. Add **PR-G-rerank (LTR cascade)** as a sibling to PR-G1 eval harness.
4. Track Qoder's Phase-3 target table (RaBitQ + LTR cascade) as the **next** scale-cycle goalpost after `scale-100M-optims` merges.

## Anti-claim

Qoder did **not** instruct or measure anything in this session. Its role is documented prior thinking, useful as a check on our roadmap. We are not implementing under Qoder's direction; we are validating that two independent roadmaps converged on the same Tier-2 architecture (HNSW + int8 + Matryoshka), which is itself useful evidence that the plan is the right one.

---
Author: hyperstack-heavy (Opus 4.7), 2026-04-23. Source quoted from `~/.qoder/specs/synapse-turbo-masterplan.md`.
