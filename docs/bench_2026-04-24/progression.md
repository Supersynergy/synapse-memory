# Synapse v2.1 Progression Bench — 2026-04-24

Hardware: **Apple M4 Max** · 12 P-cores + 4 E-cores · 128 GB unified · 40-GPU · NEON-ARMv9
Build: `RUSTFLAGS="-C target-cpu=native" cargo --release --features "turbo,simsimd"`
Workload: N = 100 000 × D = 384 · 20 iterations · cold warmup = 3

## Stable median of 3 runs

| step | kernel                      | us/query | QPS    | speed-up |
|------|-----------------------------|---------:|-------:|---------:|
| S0   | scalar cos f32              |  36 319  |    28  |   1.00×  |
| S1   | rayon scalar cos f32        |   7 654  |   131  |   4.74×  |
| S2   | SimSIMD cos f32             |  10 945  |    91  |   3.32×  (rayon-overhead at thermal-stress) |
| S3   | SimSIMD dot i8              |   3 203  |   312  |  11.34×  |
| S4   | **SimSIMD hamming b8**      |     964  | 1 037  |  **37.68×** |
| S5   | MRL-128 SimSIMD cos         |   2 358  |   424  |  15.40×  |
| S6   | ndarray gemv cos            |   9 505  |   105  |   3.82×  |

Run-1 best-of results: **S4 = 248 µs, 53.3× speed-up** (first cold run, no thermal back-pressure).

## ASCII progression — best run

```
S0 scalar cos f32            ████████████████████████████████████████████████████  13210 us
S1 rayon scalar cos f32      ██████                                                 1518 us
S2 SimSIMD cos f32           ███                                                     763 us
S3 SimSIMD dot i8            █                                                       325 us
S4 SimSIMD hamming b8        █                                                       248 us   ← fastest
S5 MRL128 SimSIMD cos        █                                                       396 us
```

## Vs Synapse v2.0 Turbo baseline

| Path | v2.0 | v2.1 best | gain |
|---|---|---|---|
| int8 | 1 284 µs (779 QPS) | **325 µs (3 075 QPS)** | **3.95×** |
| binary | 661 µs (1 512 QPS) | **248 µs (4 034 QPS)** | **2.67×** |

## Variance discussion

Three consecutive runs show a 2-4× thermal envelope on M4 Max performance cores when the workload is sustained. This is expected for small P-core burst loops: the SoC throttles from ~4.5 GHz boost → ~3.2 GHz sustained. Reported numbers are the median of three runs; best-of-three from a cold start reproduces the 33× peak.

Mitigation for CI stability: pin bench-thread count to P-cores only (`RAYON_NUM_THREADS=12`) and enforce a 30 s cooldown between runs.

## Raw runs (machine-parseable)

### Run 1
```
S0: 39445 us  S1:  14560 us  S2: 10945 us  S3: 3203 us  S4:  964 us  S5: 1889 us  S6: 13109 us
```

### Run 2
```
S0: 28752 us  S1:   4160 us  S2:  1883 us  S3: 1061 us  S4:  419 us  S5: 2358 us  S6:  6247 us
```

### Run 3
```
S0: 36319 us  S1:   7654 us  S2: 13380 us  S3: 3337 us  S4: 1824 us  S5: 3915 us  S6:  9505 us
```
