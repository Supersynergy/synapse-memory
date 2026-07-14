//! synapse-turbo: Speed optimizations for Synapse
//!
//! This module contains high-performance alternatives to the default components:
//! - `OllamaEmbedder`: 17× faster embeddings via Ollama API
//! - `NdArraySearch`: ndarray-style SIMD search (faster than sqlite-vec for <50k docs)
//! - `HybridCache`: In-memory results cache (20× faster than redb)
//!
//! Enable with feature flags:
//! - `--features turbo` for Ollama embedder
//! - ndarray_search and hybrid_cache are always available

#[cfg(feature = "ollama")]
pub mod ollama_embedder;

#[cfg(feature = "simsimd")]
pub mod simsimd_kernels;

/// f16 (half-precision) storage + conversion helpers — 50% RAM savings.
pub mod f16_kernels;

/// In-memory f16-storage brute-force index (50% RAM savings, recall ≥ 0.99).
pub mod inmem_f16_index;

/// One-call bundle — I8 + F16 + Hamming behind the AdaptiveRouter.
pub mod multi_index;

/// Candle-Metal BGE-small embedder (scaffolding; see SPEC_V2 §4 E).
pub mod candle_metal_embedder;

/// MRL-wrapping embedder decorator — Matryoshka truncate + L2 renormalize.
#[cfg(feature = "turbo")]
pub mod mrl_embedder;

pub mod hybrid_cache;
pub mod ndarray_search;
pub mod rrf_simd;

/// Thompson-bandit adaptive routing across ANN strategies.
pub mod adaptive_router;

/// In-memory int8-quantized brute-force index (SimSIMD-accelerated).
pub mod inmem_i8_index;

/// In-memory 1-bit Hamming brute-force index (SimSIMD-accelerated).
pub mod inmem_hamming_index;

/// RaBitQ rerank cascade — closes f16 recall ceiling 0.95→0.99+ via
/// scalar-quantized randomized-bit codes (Gao & Long, SIGMOD 2024).
pub mod rabitq_rerank;

/// RaBitQ cascade index — Hamming sweep → RaBitQ rerank.
pub mod rabitq_index;

/// Filtered-ANN wrapper — metadata pre/post-filter (ACORN-lite).
pub mod filtered_ann;

/// HyDE (Hypothetical Document Embedding) query augmentation via Ollama.
#[cfg(feature = "ollama")]
pub mod hyde;

/// Auto-tiered index: RAM MultiIndex + SPANN disk-tier when corpus > threshold.
/// Feature-gated `spann-tier` — without the feature, disk path compiled out.
pub mod tiered;

/// RAM optimization utilities: mlock, madvise, huge-pages, thread-local query buffers.
pub mod ram;
