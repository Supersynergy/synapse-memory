use synapse_market::store::page::{decode_page, encode_page, hilbert_index, Bar};

#[test]
fn hilbert_index_deterministic() {
    let ts_origin = 1_700_000_000_i64;
    let price_origin = 100.0_f32;
    let ts_scale = 1.0_f64;
    let price_scale = 0.01_f64;

    let h1 = hilbert_index(
        ts_origin + 100,
        105.0,
        ts_origin,
        price_origin,
        ts_scale,
        price_scale,
    );
    let h2 = hilbert_index(
        ts_origin + 100,
        105.0,
        ts_origin,
        price_origin,
        ts_scale,
        price_scale,
    );
    assert_eq!(h1, h2, "hilbert_index must be deterministic");

    let h3 = hilbert_index(
        ts_origin + 200,
        110.0,
        ts_origin,
        price_origin,
        ts_scale,
        price_scale,
    );
    // different inputs → likely different outputs (not strictly required, but a sanity check)
    let _ = h3; // just ensure it compiles and runs
}

#[test]
fn encode_page_hilbert_curve_id_set() {
    let bars: Vec<Bar> = (0..10)
        .map(|i| Bar {
            ts: 1_700_000_000 + i as i64 * 60,
            open: 100.0 + i as f32,
            high: 101.0 + i as f32,
            low: 99.0 + i as f32,
            close: 100.5 + i as f32,
            volume: 1000.0,
        })
        .collect();

    let page = encode_page(&bars);
    let (hdr, decoded) = decode_page(&page);
    assert_eq!(
        hdr.hilbert_curve_id, 1,
        "encoded page must have hilbert_curve_id=1"
    );
    assert_eq!(
        decoded.len(),
        bars.len(),
        "bar count preserved after hilbert sort"
    );
    // All original bars present (set equality by ts)
    let mut orig_ts: Vec<i64> = bars.iter().map(|b| b.ts).collect();
    let mut dec_ts: Vec<i64> = decoded.iter().map(|b| b.ts).collect();
    orig_ts.sort_unstable();
    dec_ts.sort_unstable();
    assert_eq!(orig_ts, dec_ts, "all bars must survive roundtrip");
}

#[test]
fn hilbert_sort_microsanity_10k() {
    use std::time::Instant;

    let bars: Vec<Bar> = (0..10_000)
        .map(|i| Bar {
            ts: 1_700_000_000 + i as i64 * 60,
            open: 100.0 + (i % 500) as f32 * 0.1,
            high: 101.0,
            low: 99.0,
            close: 100.0 + (i % 500) as f32 * 0.1,
            volume: 1000.0,
        })
        .collect();

    let ts_origin = bars.iter().map(|b| b.ts).min().unwrap();
    let price_origin = 100.0_f32;
    let ts_scale = 1.0_f64;
    let price_scale = 0.01_f64;

    let start = Instant::now();
    let mut keys: Vec<u64> = bars
        .iter()
        .map(|b| {
            hilbert_index(
                b.ts,
                b.close,
                ts_origin,
                price_origin,
                ts_scale,
                price_scale,
            )
        })
        .collect();
    keys.sort_unstable();
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_millis() < 10,
        "10k hilbert sort took {}ms (expected <10ms)",
        elapsed.as_millis()
    );
    assert_eq!(keys.len(), 10_000);
}
