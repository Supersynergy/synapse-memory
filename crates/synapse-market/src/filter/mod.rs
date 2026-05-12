/// Filter module.
///
/// Two filters are available:
/// - [`Bloom`]: fixed 16KB, fast to update, saturation >~100K keys → use only when `n_keys < 100_000`.
/// - [`SeriesXorFilter`]: immutable xor-filter, ~10 bits/entry, no saturation, built once at series-close.
///
/// `Series` chooses automatically: bloom for small series (<100K flushed keys), xor for large.
pub mod bloom;
pub mod xor;
pub use bloom::Bloom;
pub use xor::SeriesXorFilter;
