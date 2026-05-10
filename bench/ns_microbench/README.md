# ns_microbench — Synapse hot-path nanosecond harness

Goal: measure dist-fn ops at L1/L2/L3/DRAM tier, fail-loud on >5% regress.

## Plan

```
src/
  main.rs           — driver, mach_absolute_time, ns/op + IPC + branch-miss%
  kernels/
    f32_l2.rs       — baseline scalar
    f16_neon.rs     — NEON FMLAL2 hand-tuned
    i8_dot.rs       — int8 SDOT
    bin_hamming.rs  — POPCNT XOR (target: <0.5 ns/vec)
    amx_outer.rs    — Apple AMX block-tile (libamx) [feature-gated]
  layouts/
    aos.rs / soa.rs / blocked.rs
  workloads/
    top_k_10.rs / top_k_100.rs / top_k_1000.rs
    batch_1.rs / batch_8.rs / batch_64.rs
```

## Targets (M4 Max, fail-loud)

| Op | Target | Floor |
|----|--------|-------|
| f16 dot 128d (L1) | <2 ns | 1 ns memcpy-parity |
| f16 dot 768d (L1→L2) | <8 ns | 4 ns |
| Hamming 256-bit (L1) | <0.5 ns | 0.2 ns single-cycle |
| top-10 over 1M (cascade) | <50 µs | 5 µs cache-warm |

## Tools

- `mach_absolute_time()` for ns counter (Darwin)
- `xcrun xctrace record --template "Time Profiler"` for IPC + counters
- `cargo flamegraph` (FlameGraph + perf-record → SVG)
- `samply` cross-platform, integrates Firefox profiler
- `dtrace` syscall + cache-event probes
- `Instruments.app` → CPU Counters → L1D miss, branch-miss-rate

## CI gate

```yaml
# .github/workflows/ns-bench.yml
- run: cargo bench -p synapse-ann --bench ns_microbench -- --output-format bencher | tee bench.txt
- run: python scripts/bench_diff.py bench.txt main.txt --max-regress 5
```

## TODO order (1-week sprint)

1. baseline scalar f32 — establish floor
2. NEON FMLAL2 f16 — 2× expected
3. branchless + prefetch + cache-align — 30-50% cut
4. Hamming POPCNT bin_hamming — sub-ns target
5. cascade 1bit→8bit→f16→f32 — 95% queries finish stage 1
6. AMX outer-product prototype — moat-secure check
7. wire CI fail-loud
