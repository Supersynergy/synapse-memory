/// synapse-olap — DuckDB-embedded OLAP engine
///
/// Feature-gated: compile with `--features olap`.
/// Without the feature, only the auto-router heuristic is available (zero deps).

pub mod router;

#[cfg(feature = "olap")]
pub mod engine;

#[cfg(feature = "olap")]
pub use engine::OlapEngine;
pub use router::{Engine, is_olap, auto_route};
