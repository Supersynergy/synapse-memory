# Bloom Filter Scale Report
Generated: 2026-05-12 · Platform: Apple M4 Max · `cargo bench --release`

## TL;DR

The current fixed-size bloom filter (128 KB = 16 KB bits) **saturates** past ~100 pages.
At saturation (FPR ≈ 100 %) the filter always returns "maybe present", adding hash
overhead without any skip benefit.  Below ~50 pages the overhead dominates and bloom is
slower than plain header-scan.  The only range where bloom would help is **never** with
the current fixed size.

**Fix: auto-scale bloom bits proportional to page count, or disable bloom entirely below
200 pages and re-evaluate after resizing.**

---

## 1. Corpus

| Metric | Value |
|--------|-------|
| Tickers | 10 |
| Pages / ticker (bigscale bench) | 500 |
| Bars / ticker | 1 364 000 (500 × MAX_ROWS=2728) |
| Bar interval | 1 min |
| Bloom bits | 128 K (16 KB fixed) |
| Expected FPR @ 500 pages | ~100 % (severely saturated) |
| Disk size / ticker .smx | ~63 MB |
| Total corpus | ~630 MB |

> **Saturation math**: optimal FPR < 1 % needs ≈ 9.6 bits/element.
> At 500 pages × 2728 bars = 1 364 000 elements → 13.1 Mbit needed.
> Current allocation: 131 072 bits → 100× too small → FPR ≈ 100 %.

---

## 2. Scale-curve: neg-lookup (200 queries way after data ends)

Per-query median over 200 negative queries.

| Pages | smx_bloom (µs) | smx_noBloom (µs) | bloom faster? | ratio |
|-------|---------------|-----------------|---------------|-------|
| 20    | 2.0           | 1.6             | NO (−20%)     | 0.80× |
| 50    | 3.3           | 3.2             | NO (−3%)      | 0.97× |
| 100   | 16.5          | 16.8            | ~equal        | 1.02× |
| 200   | 73.6          | 61.0            | NO (−21%)     | 0.83× |
| 500   | 176.1         | 80.8            | NO (−55%)     | 0.46× |
| 1000  | 179.4         | 61.2            | NO (−66%)     | 0.34× |

**Crossover: none** — bloom never wins under the current fixed 128 K bits.

At ≤ 50 pages the overhead is small (sub-µs difference).  At ≥ 200 pages the saturated
bloom adds hash overhead without skipping any page, making it 2–3× **slower** than the
plain header-scan.

---

## 3. Big-scale bench @ 500 pages (1000 queries, 50% pos / 50% neg)

| Variant | Neg-lookup 500q | Pos-lookup 500q |
|---------|----------------|----------------|
| smx_bloom   | 82.1 µs | 8.72 ms |
| smx_noBloom | 81.7 µs | 7.23 ms |
| sqlite_wal  | 489.5 µs | — |

**Key findings:**
- `smx_bloom_neg` ≈ `smx_noBloom_neg`: bloom saturated → no early-exit, overhead washes out
- `smx_bloom_pos` is 20% **slower** than `smx_noBloom_pos`: bloom probe adds cost on hot path
- SMX is **6× faster** than SQLite WAL for negative lookups regardless of bloom
- Target "bloom_neg ≥ 3× faster than baseline" **not met** — saturation is the root cause

---

## 4. Root-cause

```
Bloom capacity: 128 K bits = 16 384 bytes
k-hash functions: 3
Optimal elements for FPR<1%: 128 000 / 9.6 ≈ 13 330
Actual insertions @ 100 pages: 100 × 2728 = 272 800  →  FPR ≈ 100%
```

The Bloom filter was designed for small series (few hundred bars, a handful of pages).
At 100+ pages it is completely saturated and provides **no filtering value**.

---

## 5. Recommendation

### Immediate: raise `bloom_min_pages` threshold

The `bloom_min_pages` field (added in `src/series/mod.rs`) defaults to 100.
With current 128 K bits that is still saturated, but disables the false overhead.
Change default to `usize::MAX` to **disable bloom entirely** until the filter is resized.

```rust
pub bloom_min_pages: usize,  // default: usize::MAX (disabled)
```

### Medium-term: auto-scale bloom size

Size bloom to target FPR < 1 % based on expected page count:

```
bits_needed = n_pages * MAX_ROWS * 9.6   // 9.6 bits/element for 1% FPR, k=7
```

| Pages | Bits needed | Bytes |
|-------|------------|-------|
| 50    | 1.3 M      | 163 KB |
| 100   | 2.6 M      | 327 KB |
| 500   | 13.1 M     | 1.6 MB |
| 1000  | 26.2 M     | 3.3 MB |

At ~500 pages and 1.6 MB bloom, the negative-lookup speedup should materialize at
**> 3× over header-scan** because each query becomes a single 4-probe bit-test vs.
scanning 500 index entries.

### Threshold once resized

After auto-scaling, enable bloom only at `n_pages >= 50` (bloom overhead < 100 ns vs
~50 µs scan at 50 pages).

```rust
pub bloom_min_pages: usize = 50;  // after auto-scale fix
```

---

## 6. SMX vs SQLite baseline (not bloom-dependent)

Even without bloom benefit, SMX mmap header-scan beats SQLite WAL by **6×** on
negative lookups at 500 pages.  This validates the mmap+index architecture regardless
of bloom filter status.
