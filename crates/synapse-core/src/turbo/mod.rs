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

pub mod hybrid_cache;
pub mod ndarray_search;
