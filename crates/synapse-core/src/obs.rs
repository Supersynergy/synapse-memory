//! Thin instrumentation shim — proxies to synapse-obs when `observability`
//! feature is enabled, zero-cost no-ops otherwise.

/// Record query duration. `op` ∈ {"put","search_vec","search_hybrid","search_lex"}.
#[inline]
pub fn record_query_duration(op: &str, secs: f64) {
    #[cfg(feature = "observability")]
    synapse_obs::metrics::get()
        .query_duration
        .with_label_values(&[op])
        .observe(secs);
    #[cfg(not(feature = "observability"))]
    let _ = (op, secs);
}

/// Set total document count gauge.
#[inline]
pub fn set_index_size(n: i64) {
    #[cfg(feature = "observability")]
    synapse_obs::metrics::get().index_size_docs.set(n);
    #[cfg(not(feature = "observability"))]
    let _ = n;
}

/// Record HNSW-like visited-node count (used for ndarray kNN sweep as proxy).
#[inline]
pub fn record_visited_nodes(n: u64) {
    #[cfg(feature = "observability")]
    synapse_obs::metrics::get()
        .hnsw_visited_nodes
        .observe(n as f64);
    #[cfg(not(feature = "observability"))]
    let _ = n;
}

/// Increment cache-hit counter (turbo ndarray fast-path taken).
#[inline]
pub fn inc_cache_hit() {
    #[cfg(feature = "observability")]
    synapse_obs::metrics::get().cache_hit_total.inc();
}

/// Increment cache-miss counter (cold build triggered).
#[inline]
pub fn inc_cache_miss() {
    #[cfg(feature = "observability")]
    synapse_obs::metrics::get().cache_miss_total.inc();
}
