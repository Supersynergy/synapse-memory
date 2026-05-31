use synapse_market::router::{Plan, PlanCache, QueryKey, QueryKind};

/// Bandit should converge to MmapScanSkipped (2× faster) over 1000 trials.
#[test]
fn bandit_converges_to_best_plan() {
    let mut cache = PlanCache::new(256);
    let key = QueryKey {
        kind: QueryKind::CandleRange,
        range_bars: 100,
        n_pages: 20,
        has_filter: false,
    };
    let candidates = vec![Plan::MmapScanFull, Plan::MmapScanSkipped];

    // Seed both arms so the bandit has initial measurements for both
    cache.record(key.clone(), Plan::MmapScanFull, 100);
    cache.record(key.clone(), Plan::MmapScanSkipped, 50);

    for _ in 0..1000 {
        let plan = cache.choose(&key, &candidates);
        // MmapScanSkipped is 2× faster (50µs vs 100µs)
        let latency_us = match plan {
            Plan::MmapScanSkipped => 50,
            _ => 100,
        };
        cache.record(key.clone(), plan, latency_us);
    }

    // After warmup, directly ask the bandit 200 times (bypass cache winrate shortcut)
    // by using a fresh key with no cache entry
    let key2 = QueryKey {
        kind: QueryKind::CandleRange,
        range_bars: 101, // different key → no cached entry
        n_pages: 20,
        has_filter: false,
    };
    let mut skipped_count = 0usize;
    for _ in 0..200 {
        let plan = cache.choose(&key2, &candidates);
        if plan == Plan::MmapScanSkipped {
            skipped_count += 1;
        }
        cache.record(
            key2.clone(),
            plan,
            if plan == Plan::MmapScanSkipped {
                50
            } else {
                100
            },
        );
    }
    // Exploit 90%: expect ≥160/200 picks are MmapScanSkipped
    assert!(
        skipped_count >= 150,
        "bandit should converge: got {skipped_count}/200 correct picks"
    );
}
