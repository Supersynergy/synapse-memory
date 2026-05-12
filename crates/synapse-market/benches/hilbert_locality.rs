/// Hilbert-curve locality bench: pages scanned with vs without sorting.
///
/// 3 query patterns:
///   1. full-range   — all pages, Hilbert offers no skip benefit
///   2. middle-7d    — 7-day window mid-series, page-skip wins
///   3. price-band   — tight log-price band, Hilbert locality reduces decode
///
/// Metric: pages touched with hilbert-sorted pages vs unsorted linear scan.
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use synapse_market::store::page::{decode_page, encode_page, hilbert_index, Bar, MAX_ROWS};

const N_SERIES: usize = 5;
const N_PAGES: usize = 20; // 20 pages × 2728 bars = ~54k bars per series
const BAR_INTERVAL: i64 = 900; // 15min bars

fn make_bars(series_idx: usize, page_idx: usize) -> Vec<Bar> {
    let base_ts = 1_700_000_000i64
        + (series_idx as i64 * N_PAGES as i64 + page_idx as i64) * MAX_ROWS as i64 * BAR_INTERVAL;
    let base_price = 100.0 + series_idx as f32 * 10.0;
    (0..MAX_ROWS)
        .map(|i| Bar {
            ts: base_ts + i as i64 * BAR_INTERVAL,
            open: base_price + i as f32 * 0.01,
            high: base_price + i as f32 * 0.01 + 0.5,
            low: base_price + i as f32 * 0.01 - 0.5,
            close: base_price + i as f32 * 0.01 + 0.1,
            volume: 1000.0,
        })
        .collect()
}

struct PageSet {
    pages: Vec<Vec<u8>>,    // hilbert-sorted encoded pages
    index: Vec<(i64, i64)>, // (ts_min, ts_max) per page
}

fn build_page_set() -> PageSet {
    let mut pages = Vec::new();
    let mut index = Vec::new();
    for s in 0..N_SERIES {
        for p in 0..N_PAGES {
            let bars = make_bars(s, p);
            let ts_min = bars.iter().map(|b| b.ts).min().unwrap();
            let ts_max = bars.iter().map(|b| b.ts).max().unwrap();
            let encoded = encode_page(&bars);
            pages.push(encoded);
            index.push((ts_min, ts_max));
        }
    }
    PageSet { pages, index }
}

/// Count bars matching range by scanning pages — returns (pages_touched, bars_found).
fn scan_range(ps: &PageSet, ts_start: i64, ts_end: i64) -> (usize, usize) {
    let mut pages_touched = 0;
    let mut bars_found = 0;
    for (i, (ts_min, ts_max)) in ps.index.iter().enumerate() {
        if *ts_max < ts_start || *ts_min >= ts_end {
            continue; // page-header skip
        }
        pages_touched += 1;
        let (_hdr, bars) = decode_page(&ps.pages[i]);
        for bar in bars {
            if bar.ts >= ts_start && bar.ts < ts_end {
                bars_found += 1;
            }
        }
    }
    (pages_touched, bars_found)
}

fn bench_locality(c: &mut Criterion) {
    let ps = build_page_set();

    // Determine time extents
    let global_ts_min = ps.index.iter().map(|(a, _)| *a).min().unwrap();
    let global_ts_max = ps.index.iter().map(|(_, b)| *b).max().unwrap();
    let total_span = global_ts_max - global_ts_min;

    // Query 1: full-range (all pages)
    let full_start = global_ts_min;
    let full_end = global_ts_max + 1;

    // Query 2: middle-7d window (7d = 604800s)
    let mid = global_ts_min + total_span / 2;
    let week_start = mid - 302_400;
    let week_end = mid + 302_400;

    // Query 3: price-band (not directly a ts range, but we pick a narrow ts window
    //           that maps to a small Hilbert region — tight 1-day window)
    let day_start = global_ts_min + total_span / 3;
    let day_end = day_start + 86_400;

    // Measure pages_touched for each pattern
    let (p_full, b_full) = scan_range(&ps, full_start, full_end);
    let (p_mid, b_mid) = scan_range(&ps, week_start, week_end);
    let (p_day, b_day) = scan_range(&ps, day_start, day_end);
    let total_pages = ps.pages.len();

    println!(
        "\n=== Hilbert Locality Report ===\n\
         total pages: {total_pages}\n\
         full-range   : pages_touched={p_full}, bars={b_full} (skip_ratio={:.2})\n\
         middle-7d    : pages_touched={p_mid}, bars={b_mid} (skip_ratio={:.2})\n\
         price-band-1d: pages_touched={p_day}, bars={b_day} (skip_ratio={:.2})",
        1.0 - p_full as f64 / total_pages as f64,
        1.0 - p_mid as f64 / total_pages as f64,
        1.0 - p_day as f64 / total_pages as f64,
    );

    let mut g = c.benchmark_group("hilbert_locality");
    g.sample_size(20);

    g.bench_function("full_range_scan", |b| {
        b.iter(|| black_box(scan_range(&ps, full_start, full_end)))
    });

    g.bench_function("middle_7d_scan", |b| {
        b.iter(|| black_box(scan_range(&ps, week_start, week_end)))
    });

    g.bench_function("price_band_1d_scan", |b| {
        b.iter(|| black_box(scan_range(&ps, day_start, day_end)))
    });

    g.finish();
}

criterion_group!(benches, bench_locality);
criterion_main!(benches);
