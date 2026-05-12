pub mod neon;
pub mod agg;

pub use neon::{
    mean_f32, sum_f32, min_f32, max_f32, dot_f32, correlation_f32,
    ewma_f32, rolling_mean_f32, rolling_std_f32,
};
pub use agg::{
    mean_close, mean_close_slice,
    vwap, vwap_slices,
    rolling_returns, rolling_returns_slice,
    pearson_correlation, pearson_correlation_slices,
    rolling_mean_close, rolling_mean_slice,
    rolling_std_close, rolling_std_slice,
    ewma_close, ewma_slice,
};
