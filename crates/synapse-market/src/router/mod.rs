pub mod cache;
pub mod learn;

use std::collections::HashMap;

pub use cache::PlanCache;
pub use learn::{OnlineStats, ThompsonSampler};

/// Execution plan variants the router can select.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Plan {
    MmapScanFull,
    MmapScanSkipped,
    SimdAgg,
    PageLocalAnn,
    BruteCorr,
}

/// Discriminator for query shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum QueryKind {
    CandleRange,
    Aggregate,
    Correlation,
    SimilaritySearch,
}

/// Cache key — hashed from query shape.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct QueryKey {
    pub kind: QueryKind,
    pub range_bars: usize,
    pub n_pages: usize,
    pub has_filter: bool,
}
