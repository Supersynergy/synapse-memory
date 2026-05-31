//! synapse-tsdb — time-series columnar storage (GreptimeDB-pattern lite).
//!
//! Feature-gated: compile with `--features tsdb` for Arrow/Parquet backend.
//! Without the feature, a minimal in-process columnar store is available.

#[cfg(feature = "tsdb")]
mod arrow_backend;
#[cfg(feature = "tsdb")]
pub use arrow_backend::{AggOp, TsdbStore};

pub mod fallback;
#[cfg(not(feature = "tsdb"))]
pub use fallback::{AggOp, TsdbStore};

pub use fallback::Row;
