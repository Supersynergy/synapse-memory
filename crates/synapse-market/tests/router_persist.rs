use synapse_market::router::{Plan, PlanCache, QueryKey, QueryKind};
use tempfile::TempDir;

#[test]
fn save_load_roundtrip_preserves_stats() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("plan_cache.bin");

    let key = QueryKey {
        kind: QueryKind::Aggregate,
        range_bars: 500,
        n_pages: 10,
        has_filter: true,
    };

    // Build cache with some history
    let mut cache = PlanCache::new(64);
    for _ in 0..50 {
        cache.record(key.clone(), Plan::SimdAgg, 30);
        cache.record(key.clone(), Plan::MmapScanFull, 90);
    }

    cache.save(&path).expect("save failed");

    // Load in fresh instance
    let mut loaded = PlanCache::load(&path, 64);

    // After 50 rounds SimdAgg should be preferred (30µs vs 90µs)
    let candidates = vec![Plan::SimdAgg, Plan::MmapScanFull];
    let mut simd_picks = 0usize;
    for _ in 0..50 {
        let plan = loaded.choose(&key, &candidates);
        if plan == Plan::SimdAgg {
            simd_picks += 1;
        }
        loaded.record(
            key.clone(),
            plan,
            if plan == Plan::SimdAgg { 30 } else { 90 },
        );
    }
    assert!(
        simd_picks >= 40,
        "loaded cache should prefer SimdAgg: got {simd_picks}/50"
    );
}
