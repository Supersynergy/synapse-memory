//! synapse-iouring — Linux io_uring storage engine
//!
//! # Platform Support
//!
//! | OS    | Feature flag | Status      |
//! |-------|--------------|-------------|
//! | Linux | `io-uring`   | Full (5-10× SQLite writes) |
//! | macOS | (none)       | Compile-only; runtime → `UnsupportedPlatform` |
//! | Win   | (none)       | Same as macOS |
//!
//! # Architecture (TigerBeetle pattern)
//!
//! ```text
//! append_batch(entries)
//!   → WAL (io_uring Direct-I/O, batch=32+)
//!   → L0 SkipMap (in-mem, sorted)
//!   → [compaction] SSTable files on disk
//!
//! read_range(key_range)
//!   → L0 scan (SkipMap)
//!   → [future] SSTable bloom+binary-search
//! ```

pub mod compaction;
pub mod error;
pub mod lsm;
pub mod store;

#[cfg(feature = "io-uring")]
pub mod uring;

pub use compaction::{CompactCmd, Compactor, TieredConfig};
pub use error::IoUringError;
pub use lsm::{BloomFilter, Entry, Key};
pub use store::IoUringStore;

mod tests;
