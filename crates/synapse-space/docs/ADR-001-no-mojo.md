# ADR-001: Skip Mojo for synapse-space kernel layer

**Status**: Accepted  
**Date**: 2026-05-03  
**Deciders**: Maxim Supersynergy  

## Decision

Do not use Mojo for any part of synapse-space or synapse-core hot paths.

## Context

Mojo was evaluated as a potential replacement for Rust in the embedding and
vector-search kernel layer, primarily for its claimed GPU/Metal affinity and
Python superset syntax.

## Rationale

1. **MLX-Metal already optimal on M4 Max**: Current embed latency is 2.45 ms
   (Phase-5 MLX path). Mojo provides no measurable gain on Apple Silicon
   because MLX already targets Metal natively. Adding a second kernel language
   for zero throughput delta is pure complexity debt.

2. **Mojo Metal backend not production-ready (early 2026)**: Mojo's Metal
   backend is in active development. Relying on it for a production binary
   introduces an unstable ABI dependency with no stability guarantees.

3. **CUDA-first ecosystem**: Mojo's primary optimization target is NVIDIA CUDA.
   This project's hardware baseline is M4 Max + Apple Silicon. The Mojo
   ecosystem is optimised for a target we don't run.

4. **No cross-compile requirement**: synapse-space is a single-host embedded
   engine. There is no need for the Python-superset portability Mojo offers.
   Single Rust binary via `cargo build --release` covers all deployment targets.

5. **Rust + MLX = current local optimum**: The Rust + pyo3 + MLX-Metal stack
   provides sub-3 ms embedding, 44k FTS5 ops/s, and sub-ms vector search —
   all measured in production. Mojo cannot improve on any of these numbers
   on the current hardware.

## Re-evaluate triggers

- Target shifts to NVIDIA GPU server (Mojo CUDA > Rust CUDA ergonomics).
- Mojo ships first-class stable Metal backend with measured speedup on M-series.
- Kernel-level batch ops (e.g. fused embed+index) dominate the latency profile
  and Mojo's compiler proves faster than Rust + SIMD intrinsics on those ops.
- Mojo gains a first-class Rust FFI ABI (currently Python-only interop).

## Consequences

- Rust remains the single systems language for all hot paths.
- MLX-Metal via `synapse-metal` crate handles embedding; no Python runtime in
  the binary distribution path.
- Mojo skills/proofs-of-concept are welcome in `bench/` or `examples/` but
  will not land in `crates/`.
