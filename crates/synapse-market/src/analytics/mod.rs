pub mod agg;
pub mod amx;
pub mod neon;

pub use amx::{correlation_matrix_amx, CorrMatrix};

pub use agg::{
    agg_pages, ewma_close, ewma_slice, mean_close, mean_close_slice, pearson_correlation,
    pearson_correlation_slices, rolling_mean_close, rolling_mean_slice, rolling_returns,
    rolling_returns_slice, rolling_std_close, rolling_std_slice, vwap, vwap_slices, AggKind,
    AggResult,
};
pub use neon::{
    correlation_f32, dot_f32, ewma_f32, max_f32, mean_f32, min_f32, rolling_mean_f32,
    rolling_std_f32, sum_f32,
};
