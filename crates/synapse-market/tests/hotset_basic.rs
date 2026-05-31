use synapse_market::cache::{DecodedPage, HotSet, PageKey};
use synapse_market::series::Series;
use synapse_market::store::page::Bar;
use tempfile::TempDir;

const BASE_TS: i64 = 1_700_000_000;

fn bar(i: usize) -> Bar {
    Bar {
        ts: BASE_TS + i as i64 * 900,
        open: 10.0,
        high: 11.0,
        low: 9.0,
        close: 10.5,
        volume: 500.0,
    }
}

// ── HotSet unit tests ─────────────────────────────────────────────────────────

#[test]
fn hotset_insert_and_lookup() {
    let mut hs = HotSet::new(10);
    let k = PageKey {
        series_id: 1,
        page_idx: 0,
    };
    let page = hs.get_or_load(k.clone(), || DecodedPage {
        ts: vec![1, 2, 3],
        close: vec![1.0, 2.0, 3.0],
        volume: None,
        bars: vec![],
    });
    assert_eq!(page.ts, vec![1, 2, 3]);
    let (hits, misses) = hs.stats();
    assert_eq!(hits, 0);
    assert_eq!(misses, 1);
}

#[test]
fn hotset_hit_counter() {
    let mut hs = HotSet::new(10);
    let k = PageKey {
        series_id: 42,
        page_idx: 7,
    };
    hs.get_or_load(k.clone(), || DecodedPage {
        ts: vec![],
        close: vec![],
        volume: None,
        bars: vec![],
    });
    hs.get_or_load(k.clone(), || panic!("should not call loader on hit"));
    let (hits, misses) = hs.stats();
    assert_eq!(hits, 1);
    assert_eq!(misses, 1);
}

#[test]
fn hotset_eviction() {
    let mut hs = HotSet::new(2);
    for i in 0u32..3 {
        let k = PageKey {
            series_id: 0,
            page_idx: i,
        };
        hs.get_or_load(k, || DecodedPage {
            ts: vec![i as i64],
            close: vec![i as f32],
            volume: None,
            bars: vec![],
        });
    }
    let (_, misses) = hs.stats();
    assert_eq!(misses, 3); // all 3 were misses (cap=2, oldest evicted)
}

// ── Series point_lookup tests ─────────────────────────────────────────────────

#[test]
fn point_lookup_cold() {
    let dir = TempDir::new().unwrap();
    let mut s = Series::open(dir.path().join("a.smx")).unwrap();
    let bars: Vec<Bar> = (0..50).map(bar).collect();
    s.append(&bars).unwrap();
    s.flush_pending().unwrap();

    let found = s.point_lookup(BASE_TS + 5 * 900).unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().ts, BASE_TS + 5 * 900);

    let missing = s.point_lookup(BASE_TS - 1).unwrap();
    assert!(missing.is_none());
}

#[test]
fn point_lookup_hot() {
    let dir = TempDir::new().unwrap();
    let mut s = Series::open(dir.path().join("b.smx")).unwrap();
    let bars: Vec<Bar> = (0..50).map(bar).collect();
    s.append(&bars).unwrap();
    s.flush_pending().unwrap();
    s.enable_hot_cache(100);

    // first call = miss
    let r1 = s.point_lookup(BASE_TS + 10 * 900).unwrap();
    assert!(r1.is_some());
    // second call = hit
    let r2 = s.point_lookup(BASE_TS + 10 * 900).unwrap();
    assert_eq!(r1.unwrap().ts, r2.unwrap().ts);

    let (hits, misses) = s.hot.as_ref().unwrap().stats();
    assert!(hits >= 1, "expected ≥1 hit, got {hits}");
    assert!(misses >= 1, "expected ≥1 miss, got {misses}");
}
