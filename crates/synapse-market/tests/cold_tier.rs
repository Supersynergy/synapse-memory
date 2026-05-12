use synapse_market::store::compact::{cold_stats, ColdTier};
use synapse_market::store::page::{decode_page, encode_page, Bar, PAGE_SIZE};
use tempfile::TempDir;

fn make_bars(n: usize, base_ts: i64) -> Vec<Bar> {
    (0..n)
        .map(|i| Bar {
            ts: base_ts + i as i64 * 900,
            open: 100.0 + i as f32 * 0.1,
            high: 101.0 + i as f32 * 0.1,
            low: 99.0 + i as f32 * 0.1,
            close: 100.5 + i as f32 * 0.1,
            volume: 1000.0 + i as f32,
        })
        .collect()
}

fn make_raw_pages(n_pages: usize, base_ts: i64) -> Vec<Vec<u8>> {
    (0..n_pages)
        .map(|p| {
            let bars = make_bars(100, base_ts + p as i64 * 100 * 900);
            encode_page(&bars)
        })
        .collect()
}

#[test]
fn cold_roundtrip_identity() {
    let dir = TempDir::new().unwrap();
    let csm = dir.path().join("test.csm");

    let base = 1_700_000_000i64;
    let raw_pages = make_raw_pages(100, base);
    let original_bars: Vec<Bar> = raw_pages
        .iter()
        .flat_map(|p| {
            let (_, bars) = decode_page(p);
            bars
        })
        .collect();

    ColdTier::create(&csm, &raw_pages).expect("create cold tier");

    let mut cold = ColdTier::open(&csm).expect("open cold tier");
    assert_eq!(cold.page_count(), 100);

    let mut recovered: Vec<Bar> = Vec::new();
    for i in 0..100u32 {
        let bars = cold.read_page(i).expect("read page");
        recovered.extend(bars);
    }

    assert_eq!(recovered.len(), original_bars.len());
    for (a, b) in original_bars.iter().zip(recovered.iter()) {
        assert_eq!(a.ts, b.ts);
        assert!(
            (a.close - b.close).abs() < 1e-5,
            "close mismatch at ts={}",
            a.ts
        );
        assert!((a.open - b.open).abs() < 1e-5);
    }
}

#[test]
fn decompress_cache_warms() {
    let dir = TempDir::new().unwrap();
    let csm = dir.path().join("cache_test.csm");

    let raw_pages = make_raw_pages(10, 1_700_000_000);
    ColdTier::create(&csm, &raw_pages).unwrap();

    let mut cold = ColdTier::open(&csm).unwrap();
    assert_eq!(cold.cache_len(), 0, "cache starts empty");

    // Read 5 pages
    for i in 0..5u32 {
        cold.read_page(i).unwrap();
    }
    assert_eq!(cold.cache_len(), 5, "cache has 5 entries after 5 reads");

    // Re-read same pages — hits
    for i in 0..5u32 {
        cold.read_page(i).unwrap();
    }
    assert_eq!(cold.cache_len(), 5, "cache size unchanged on hits");
}

#[test]
fn hot_only_access_without_cold_file() {
    use synapse_market::series::Series;
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("hot_only.smx");

    let mut s = Series::open(&path).unwrap();
    let base = 1_700_000_000i64;
    let bars = make_bars(300, base);
    s.append(&bars).unwrap();
    s.flush_pending().unwrap();

    // No .csm file exists — range should still work
    let result = s.range(base..base + 300 * 900).unwrap();
    assert_eq!(result.len(), 300);
}

#[test]
fn compact_to_cold_and_range() {
    use synapse_market::series::Series;
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("compact.smx");

    let mut s = Series::open(&path).unwrap();
    // Use very old timestamps so age_days=0 compacts everything
    let base = 1_000_000_000i64; // year 2001
    let bars = make_bars(300, base);
    s.append(&bars).unwrap();
    s.flush_pending().unwrap();

    let n_compacted = s.compact_to_cold(0).expect("compact");
    assert!(n_compacted > 0, "should compact at least one page");

    // range should still return all bars (from cold tier)
    let result = s.range(base..base + 300 * 900).unwrap();
    assert_eq!(result.len(), 300, "all bars readable after compaction");

    // Timestamps should be sorted
    for w in result.windows(2) {
        assert!(w[0].ts <= w[1].ts);
    }
}

#[test]
fn cold_compression_ratio() {
    let dir = TempDir::new().unwrap();
    let csm = dir.path().join("ratio.csm");

    let raw_pages = make_raw_pages(20, 1_700_000_000);
    ColdTier::create(&csm, &raw_pages).unwrap();

    let stats = cold_stats(&csm).unwrap();
    let ratio = stats.compression_ratio();
    // Financial OHLCV data typically compresses 4-8× with zstd-19
    // Our synthetic test data (arithmetic progression) compresses very well
    assert!(
        ratio >= 2.0,
        "compression ratio {ratio:.2} below 2× (expected 4-8×)"
    );
    println!("cold compression ratio: {ratio:.2}×");
    println!(
        "hot bytes: {} cold bytes: {} savings: {:.1}%",
        stats.hot_bytes,
        stats.total_compressed_bytes,
        (1.0 - 1.0 / ratio) * 100.0
    );
}
