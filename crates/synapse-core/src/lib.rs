//! synapse-core: single-file memory store on SQLite+FTS5+sqlite-vec.

pub mod backend;
pub mod crdt;
pub mod db;
pub mod error;
pub mod federate;
pub mod shard;
pub mod sign;
pub mod snap;
pub mod types;

#[cfg(any(feature = "embed", feature = "embed-dynamic"))]
pub mod embed;

#[cfg(feature = "turbo")]
pub mod turbo;

/// Unified `TextEmbedder` trait + backend selector (`fastembed` / `ollama` / future MLX).
#[cfg(feature = "turbo")]
pub mod embedder_trait;

/// Matryoshka (MRL) embedding truncation — 3–6× matvec speed-up, near-full recall.
pub mod matryoshka;

pub mod sql_fns;

/// SOTA agent-memory layer: typed memories, entity graph, multi-signal recall.
/// Additive — call `sota::sota_migrate(&store.conn)` once to enable.
pub mod sota;

/// Lightweight NER (gazetteer + regex tier) for entity-aware recall.
pub mod sota_ner;

/// SOTA pipeline glue: query decomposition + Self-RAG + HyDE + evolve/compact.
pub mod sota_pipeline;

/// Personalized PageRank over `memory_edges` (HippoRAG-2 retrieval signal).
pub mod ppr;

/// MLX Metal embedder scaffold (Apple Silicon, Phase 5 Day 57-65).
#[cfg(all(target_os = "macos", target_arch = "aarch64", feature = "embed-mlx"))]
pub mod embed_mlx;

/// PR-A1-wire: usearch ANN fast-path behind feature `ann-usearch`
/// (default OFF per SPEC §3 pure-Rust-default rule).
#[cfg(feature = "ann-usearch")]
pub mod ann;

pub use db::Store;
pub use error::{Error, Result};
pub use types::{Doc, Hit, PutRequest, SearchMode};
