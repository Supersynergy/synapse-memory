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

pub use db::Store;
pub use error::{Error, Result};
pub use types::{Doc, Hit, PutRequest, SearchMode};
