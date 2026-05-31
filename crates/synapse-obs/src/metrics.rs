//! Prometheus metric definitions — shared global registry.

use prometheus::{
    Histogram, HistogramOpts, HistogramVec, IntCounter, IntGauge, Opts, Registry,
    exponential_buckets,
};
use std::sync::OnceLock;

static REGISTRY: OnceLock<Registry> = OnceLock::new();
static METRICS: OnceLock<Metrics> = OnceLock::new();

pub fn registry() -> &'static Registry {
    REGISTRY.get_or_init(Registry::new)
}

pub fn get() -> &'static Metrics {
    METRICS.get_or_init(|| Metrics::new(registry()).expect("metric registration failed"))
}

pub struct Metrics {
    /// Histogram: query wall-clock latency in seconds.
    /// Labels: `op` ∈ {put, search_vec, search_hybrid, search_lex}
    pub query_duration: HistogramVec,
    /// Gauge: total number of indexed documents.
    pub index_size_docs: IntGauge,
    /// Histogram: HNSW nodes visited per query (proxy when ann-usearch enabled).
    pub hnsw_visited_nodes: Histogram,
    /// Counter: ndarray cache hits (turbo fast-path).
    pub cache_hit_total: IntCounter,
    /// Counter: ndarray cache misses (cold or evicted).
    pub cache_miss_total: IntCounter,
}

impl Metrics {
    fn new(reg: &Registry) -> prometheus::Result<Self> {
        let query_duration = HistogramVec::new(
            HistogramOpts::new(
                "synapse_query_duration_seconds",
                "Query wall-clock latency in seconds",
            )
            .buckets(exponential_buckets(0.0001, 2.0, 16)?),
            &["op"],
        )?;
        reg.register(Box::new(query_duration.clone()))?;

        let index_size_docs = IntGauge::with_opts(Opts::new(
            "synapse_index_size_docs",
            "Total documents in the synapse store",
        ))?;
        reg.register(Box::new(index_size_docs.clone()))?;

        let hnsw_visited_nodes = Histogram::with_opts(
            HistogramOpts::new(
                "synapse_hnsw_visited_nodes",
                "HNSW candidate nodes visited per search_vec call",
            )
            .buckets(exponential_buckets(1.0, 2.0, 20)?),
        )?;
        reg.register(Box::new(hnsw_visited_nodes.clone()))?;

        let cache_hit_total = IntCounter::with_opts(Opts::new(
            "synapse_cache_hit_total",
            "ndarray turbo-cache hits (fast kNN path taken)",
        ))?;
        reg.register(Box::new(cache_hit_total.clone()))?;

        let cache_miss_total = IntCounter::with_opts(Opts::new(
            "synapse_cache_miss_total",
            "ndarray turbo-cache misses (cold build triggered)",
        ))?;
        reg.register(Box::new(cache_miss_total.clone()))?;

        Ok(Self {
            query_duration,
            index_size_docs,
            hnsw_visited_nodes,
            cache_hit_total,
            cache_miss_total,
        })
    }
}
