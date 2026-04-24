# Task #1 — Custom Metal shader for int8 MatVec (deferred planning doc)

**Status**: Planned. Not shipped in v2.1-m4max-preview. Ship target: v2.2.

## Why deferred

- `SimSIMD int8 dot` (task #2 equivalent) already at **348 µs / 2 877 QPS / 36.9× scalar** on M4 Max without a custom Metal shader.
- An `objc2-metal`-based compute pipeline adds **250-300 LOC**, a second build-system concern (`.metal` shaders + `xcrun metal` tooling), and a platform-exclusive feature-gate path.
- Expected win is **1.2-1.6×** over the existing SIMSIMD path at our scale —
  not breakthrough territory, and below the gap we'd cross by finishing
  lever #6 (Product Quantization) or wiring the existing HNSW-PQ feature.

## Planned scaffold (when picked up)

1. **Crate**: new `synapse-metal` workspace member, feature `metal`.
2. **Deps**:
   ```
   objc2      = "0.5"
   objc2-metal = "0.2"
   half       = "2.4"   # already in synapse-core
   ```
3. **Kernel MSL** (`kernels/int8_matvec.metal`):
   ```metal
   kernel void int8_matvec(
       device const int8_t *codes      [[buffer(0)]],
       device const float  *scales     [[buffer(1)]],
       device const int8_t *query      [[buffer(2)]],
       device const float  *q_scale    [[buffer(3)]],
       constant uint       &dim        [[buffer(4)]],
       device float        *scores     [[buffer(5)]],
       uint                 row        [[thread_position_in_grid]])
   {
       int acc = 0;
       for (uint d = 0; d < dim; ++d) {
           acc += (int)codes[row * dim + d] * (int)query[d];
       }
       scores[row] = (float)acc * scales[row] * (*q_scale);
   }
   ```
4. **Rust pipeline**:
   - `MTLDevice::system_default`
   - compile MSL via `newLibraryWithSource:options:error:`
   - cache `MTLComputePipelineState`
   - per-query: three `MTLBuffer` setup → `commandBuffer.encode` → commit → wait.
5. **Bench line**: S9 in `bench_progression.rs`, expected 100-150 µs at 100 k × 384.

## Gate before picking up

Only worth it when either is true:

- **Corpus > 5 M**: at that scale SimSIMD CPU starts falling behind GPU.
- **Batched queries**: Metal compute dispatch overhead amortizes across ≥ 8 queries in a batch.
- **Power budget**: on-battery tests where CPU 2.5 GHz limits bite (Metal runs on the unified 40-GPU at ~1 GHz but with 3× the lanes).

Until one fires, the SIMSIMD path is the right default. This doc + task #1
stay open as signposts.

## Risk list

- `objc2-metal` v0.2 ABI still pre-1.0. A breaking release mid-dev costs a day.
- MSL compilation at runtime adds 10-30 ms per process start. Cache the
  `MTLLibrary` to disk (via `binaryArchive`) to avoid the hit.
- Thread-group size tuning per M-series generation (M1 vs M4) — need a
  `config.json` shipped with the crate.

## References

- Apple Metal Shading Language Specification 2.5+
- `mlx-rs` (oxiglade/mlx-rs) — precedent for a Rust-wrapped Metal-backed tensor lib
- RuVector `crates/ruvllm/src/metal/pipelines.rs` — precedent for multi-kernel compute pipeline management in Rust
