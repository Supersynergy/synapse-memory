# RaBitQ Rotation — Negative Result

**Date**: 2026-05-05  
**Corpus**: synthetic 10k × 64, uniform random normalized vectors  
**binary_k**: 128, K=10, 100 queries  

## Results

| Setup | R@10 |
|-------|------|
| No rotation (baseline) | 0.644 |
| RaBitQ rotation (Gaussian GS) | 0.473 |
| **Delta** | **−0.171 (−27%)** |

## Why Rotation Hurts Here

RaBitQ's benefit (from Gao et al. 2024) relies on:
1. High-dimensional vectors (d ≥ 384) where random projections become isotropic
2. Structured/clustered data where original coordinates are NOT uniform

Our cascade operates on **already-normalized, uniform-random vectors** in d=64.
Sign(v) for uniform unit vectors already has ~50% positive bits per dimension,
meaning the binary sketch is well-distributed without rotation.
Rotation destroys any residual structure rather than improving bit entropy.

## When Rotation Would Help

- Clustered corpora where most bits cluster on one sign (e.g. embedding models
  that produce vectors with systematic positive/negative biases per dimension)
- High-dim (d ≥ 256) where Gaussian rotation provides near-isometry

## Root Cause of Implementation

The Gram-Schmidt rotation was implemented correctly (Box-Muller N(0,1) samples,
proper column orthonormalization). The math is correct; the assumption is wrong
for this data distribution.

## Recommendation

**Skip**. Do not merge. The unrotated cascade achieves R@10 ≥ 0.99 at binary_k=4096
on the production 164k×384 BGE corpus. Adding rotation overhead (O(N·D²) extra FLOP
for the matrix multiply at index time) for negative recall delta is not justified.

Revisit if: (a) a real corpus shows systematic bit-bias in binary sketches, or
(b) Synapse adopts quantization-aware embedders that produce non-isotropic unit vectors.
