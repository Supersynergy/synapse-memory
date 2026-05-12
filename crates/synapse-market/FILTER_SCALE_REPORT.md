# Filter Scale Report (Bloom → Xor Migration)
Generated: 2026-05-12 · Platform: Apple M4 Max · `cargo bench --release`

## TL;DR

Bloom filter saturated at ~100K keys (fixed 16KB, FPR→100%). Replaced with `xorf::Xor8`
for large series (≥100K keys). Xor-filter does not saturate, uses ~10 bits/entry, and is
1.3× faster per-lookup than bloom at 256K keys.

---

## 1. Bloom saturation (original findings)

| Pages | Bloom neg-lookup | No-filter | Bloom faster? |
|-------|-----------------|-----------|---------------|
| 20    | 2.0 µs          | 1.6 µs    | NO (0.80×)    |
| 50    | 3.3 µs          | 3.2 µs    | NO (0.97×)    |
| 100   | 16.5 µs         | 16.8 µs   | ~equal        |
| 200   | 73.6 µs         | 61.0 µs   | NO (0.83×)    |
| 500   | 176.1 µs        | 80.8 µs   | NO (0.46×)    |

Root-cause: 128K bits / 9.6 bits per element → saturates at ~13K keys. Past that FPR→100%.

---

## 2. Xor-filter bench @ 500 pages (256K keys, 1000 neg-probes)

`cargo bench --bench xor_vs_bloom -- --sample-size 10`

| Filter         | p50 (1000 probes) | Per-key   | vs bloom |
|----------------|-------------------|-----------|----------|
| xorf (Xor8)    | 1.68 µs           | 1.68 ns   | 1.0×     |
| bloom (16KB)   | 2.18 µs           | 2.18 ns   | 0.77× (slower) |
| no-filter (hdr-scan) | 124 ns       | n/a       | simulated 500-entry scan |
| sqlite-WAL     | 2.08 µs           | n/a       | n/a      |

**xorf is 1.3× faster than bloom per hash-lookup at 256K keys.**

Note: the no-filter "124ns" is a tight 500-integer comparison loop (not page-decode).
In real queries each page-decode is ~µs; xorf guards avoid all decodes on negative lookups.

---

## 3. Xor size vs bloom

| Keys   | Xor8 bytes | Bloom bytes | Ratio |
|--------|-----------|-------------|-------|
| 100K   | ~125 KB   | 16 KB       | 7.8×  |
| 256K   | ~320 KB   | 16 KB (saturated) | 20× |
| 1.4M   | ~1.75 MB  | 16 KB (100% FPR) | 109× |

Xor is larger but functional at all scales. Bloom is useless past 13K keys with fixed 16KB.

---

## 4. Production routing (series/mod.rs)

```rust
const XOR_THRESHOLD: usize = 100_000;  // keys in flush_chunk
// close(): n_keys >= threshold → build+persist .xor sidecar
// open(): .xor exists → load SeriesXorFilter → xor_range_likely() guard
// fallback: bloom for small series + bloom_min_pages gate
```

Sidecar files:
- `<series>.bloom` — legacy, small series only
- `<series>.xor` — new, large series (≥100K ts keys)

---

## 5. SMX vs SQLite baseline

SMX mmap header-scan beats SQLite WAL by ~6× on negative lookups regardless of filter.
With xor guard active, large-series neg-lookups become O(1) hash probe instead of O(pages).
