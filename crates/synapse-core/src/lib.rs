//! synapse-core: single-file memory store on SQLite+FTS5+sqlite-vec.

pub mod crdt;
pub mod db;
pub mod error;
pub mod federate;
pub mod shard;
pub mod sign;
pub mod snap;
pub mod types;

#[cfg(feature = "embed")]
pub mod embed;

#[cfg(feature = "turbo")]
pub mod turbo;

/// PR-A1-wire: usearch ANN fast-path behind feature `ann-usearch`
/// (default OFF per SPEC §3 pure-Rust-default rule).
#[cfg(feature = "ann-usearch")]
pub mod ann;

pub use db::Store;
pub use error::{Error, Result};
pub use types::{Doc, Hit, PutRequest, SearchMode};
