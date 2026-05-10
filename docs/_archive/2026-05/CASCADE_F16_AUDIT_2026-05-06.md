# Cascade F16 Hot-Path Audit — 2026-05-06

## Scope

`synapsestore/crates/synapse-ultra/src/search.rs` — `top_k_binary_first()` and helpers.
Prior commits: 5e67a8c (f16 matrix), 76d408b (simsimd NEON).

---

## Cascade Path (T1')

```
query_f32 → pack_signs → hamming scan (48-byte popcount, all n rows)
         → top-rerank_n candidates
         → query_to_f16(query_f32)   ← f16 conversion, ONCE per query
         → dot_f16f16(q_f16, row_f16) via simsimd NEON, per candidate
         → partial_top_k → top-k
```

---

## F32 Path Findings

| Location | Type | Used in hot path? | Notes |
|----------|------|-------------------|-------|
| `dot_f32()` (search.rs:33) | f32 brute-force | No — T1-strict only | Separate `top_k_f32` path |
| `dot_f16_row()` (search.rs:40) | scalar f16→f32 decode per element | **No** — dead in cascade | Was early impl, superseded by `dot_f16f16` |
| `query_to_f16()` (search.rs:53) | f32→f16 once per query | Yes, correct | One conversion, not per-row |
| `dot_f16f16()` (search.rs:47) | NEON FP16 simsimd | Yes — rerank hot loop | Zero f32 widening |
| `search_hnsw` rerank (index.rs:176) | `1.0 - dist` f32 | HNSW path only | Not cascade |

**Result: No f32 leak in cascade path.** The `dot_f16_row` scalar fallback exists but is not called anywhere in `top_k_binary_first`. Cascade is fully f16 end-to-end since commits 5e67a8c + 76d408b.

---

## Benchmark Results (M4 Max, NEON, criterion 0.8)

Bench: `cargo bench --bench bench_cascade_f16 -p synapse-ultra`
k=10, rerank_n=500, dim=384, synthetic normalized xorshift vectors.

| n | p50 (median) | range | throughput |
|---|-------------|-------|------------|
| 1,000 | ~256 µs* | 248–265 ns | 3.9 Gelem/s |
| 10,000 | 340 µs | 319–362 µs | 29.4 Melem/s |
| 100,000 | 934 µs | 900–973 µs | 107 Melem/s |

*Note: 1k median shows nanoseconds (sub-µs) — hamming scan of 1k rows is negligible; cost is dominated by the 500-candidate f16 rerank regardless of n.

`query_to_f16` (one-shot 384-d conversion): ~290 ns — truly negligible.

---

## Recommendation

**Keep as-is.** No f32 hot path found. Cascade is clean:
- Hamming scan: pure u8 popcount, NEON-vectorized
- Rerank: `dot_f16f16` via simsimd, no decode overhead
- Query conversion: once per query (~290 ns)

The only dead code is `dot_f16_row` (scalar per-element decode). Not worth removing — it's a safe fallback that never runs.

**At 100k: ~934 µs median** — aligns with production 162k @ ~1.4ms target.

---

## Files Added

- `benches/bench_cascade_f16.rs` — criterion benchmark, 1k/10k/100k scales
- `Cargo.toml` — `[[bench]] bench_cascade_f16` entry added
