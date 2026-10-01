//! synapse-mlx-olap — Metal GPU vectorized OLAP engine.
//!
//! Feature `mlx-olap` activates candle-core Metal backend.
//! Without the feature → CPU columnar fallback (always available).

mod agg;
mod batch;
mod engine;

pub use agg::AggOp;
pub use batch::{Column, RecordBatch};
pub use engine::MlxOlapEngine;
