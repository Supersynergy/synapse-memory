/// Property-based tests for page encode/decode, range-filter, delta-encoding.
use proptest::prelude::*;
use synapse_market::store::page::{decode_page, encode_page, Bar};

// ── Strategies ────────────────────────────────────────────────────────────────
fn arb_bar() -> impl Strategy<Value = Bar> {
    (
        0i64..=10_000_000_000i64,
        0.01f32..=10_000.0f32,
        0.01f32..=10_000.0f32,
        0.01f32..=10_000.0f32,
        0.01f32..=10_000.0f32,
        0.0f32..=1_000_000_000.0f32,
    )
        .prop_map(|(ts, open, high, low, close, volume)| Bar {
            ts,
            open,
            high,
            low,
            close,
            volume,
        })
}

fn arb_bars(max: usize) -> impl Strategy<Value = Vec<Bar>> {
    proptest::collection::vec(arb_bar(), 1..=max)
}

proptest! {
    // Property 1: roundtrip identity — decode(encode(bars)) == bars
    #[test]
    fn roundtrip_identity(bars in arb_bars(1024)) {
        let encoded = encode_page(&bars);
        let (hdr, decoded) = decode_page(&encoded);
        prop_assert_eq!(hdr.row_count as usize, bars.len());
        prop_assert_eq!(decoded.len(), bars.len());
        for (a, b) in bars.iter().zip(decoded.iter()) {
            prop_assert_eq!(a.ts, b.ts);
            prop_assert!((a.open   - b.open  ).abs() < 1e-6, "open   mismatch: {} vs {}", a.open,   b.open);
            prop_assert!((a.high   - b.high  ).abs() < 1e-6, "high   mismatch: {} vs {}", a.high,   b.high);
            prop_assert!((a.low    - b.low   ).abs() < 1e-6, "low    mismatch: {} vs {}", a.low,    b.low);
            prop_assert!((a.close  - b.close ).abs() < 1e-6, "close  mismatch: {} vs {}", a.close,  b.close);
            prop_assert!((a.volume - b.volume).abs() < 1e-6, "volume mismatch: {} vs {}", a.volume, b.volume);
        }
    }

    // Property 2: range-filter monotonicity — result is sorted by ts
    #[test]
    fn range_filter_sorted(bars in arb_bars(256), ts_a in 0i64..5_000_000_000i64) {
        use tempfile::TempDir;
        use synapse_market::series::Series;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("prop.smx");
        let mut s = Series::open(&path).unwrap();
        s.append(&bars).unwrap();
        s.flush_pending().unwrap();

        let ts_b = ts_a + 1_000_000;
        let result = s.range(ts_a..ts_b).unwrap();

        // Monotonically non-decreasing
        for w in result.windows(2) {
            prop_assert!(w[0].ts <= w[1].ts,
                "not sorted: {} > {}", w[0].ts, w[1].ts);
        }
        // All within range
        for bar in &result {
            prop_assert!(bar.ts >= ts_a && bar.ts < ts_b,
                "bar ts {} out of range [{}, {})", bar.ts, ts_a, ts_b);
        }
    }

    // Property 3: delta-encoding invertibility (manual: ts delta round-trip)
    // Since pages store ts as absolute i64, verify that min/max survive encode/decode
    #[test]
    fn delta_ts_invertible(bars in arb_bars(512)) {
        let expected_min = bars.iter().map(|b| b.ts).min().unwrap();
        let expected_max = bars.iter().map(|b| b.ts).max().unwrap();
        let encoded = encode_page(&bars);
        let (_, decoded) = decode_page(&encoded);
        let got_min = decoded.iter().map(|b| b.ts).min().unwrap();
        let got_max = decoded.iter().map(|b| b.ts).max().unwrap();
        prop_assert_eq!(expected_min, got_min);
        prop_assert_eq!(expected_max, got_max);
    }

    // Property 4: page size is always PAGE_SIZE
    #[test]
    fn page_size_deterministic(bars in arb_bars(1024)) {
        use synapse_market::store::page::PAGE_SIZE;
        let encoded = encode_page(&bars);
        prop_assert_eq!(encoded.len(), PAGE_SIZE,
            "expected PAGE_SIZE={PAGE_SIZE} got {}", encoded.len());
    }

    // Property 5: price-filter subset of full range
    #[test]
    fn price_filter_is_subset(bars in arb_bars(256), p_lo in 0.01f32..=5000.0f32) {
        use tempfile::TempDir;
        use synapse_market::series::Series;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("prop2.smx");
        let mut s = Series::open(&path).unwrap();
        s.append(&bars).unwrap();
        s.flush_pending().unwrap();

        let full = s.range(i64::MIN / 2..i64::MAX / 2).unwrap();
        // price-filter manually (Series::range returns all, filter here)
        let filtered: Vec<Bar> = s.range(i64::MIN / 2..i64::MAX / 2).unwrap()
            .into_iter()
            .filter(|b| b.close >= p_lo && b.close < p_lo * 2.0)
            .collect();

        prop_assert!(filtered.len() <= full.len());
        // Every filtered bar must appear in full (by ts)
        let full_ts: std::collections::HashSet<i64> = full.iter().map(|b| b.ts).collect();
        for bar in &filtered {
            prop_assert!(full_ts.contains(&bar.ts));
            prop_assert!(bar.close >= p_lo && bar.close < p_lo * 2.0,
                "bar.close={} not in [{p_lo}, {})", bar.close, p_lo * 2.0);
        }
    }
}
