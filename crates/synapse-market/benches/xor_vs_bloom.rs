/// Bench: xor-filter vs bloom vs no-filter vs sqlite-WAL on 500-page corpus.
/// Target: xorf ≥3× faster than no-filter on negative lookups.
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use synapse_market::filter::{Bloom, SeriesXorFilter};
use synapse_market::series::Series;
use synapse_market::store::page::Bar;
use tempfile::TempDir;
use xxhash_rust::xxh3::xxh3_64;

fn make_bars(n: usize, base_ts: i64) -> Vec<Bar> {
    (0..n)
        .map(|i| Bar {
            ts: base_ts + i as i64 * 900,
            open: 100.0,
            high: 101.0,
            low: 99.0,
            close: 100.5,
            volume: 1000.0,
        })
        .collect()
}

// Build a 500-page series (~256K bars) and return the dir + path
fn build_500_page_corpus() -> (TempDir, std::path::PathBuf, Vec<u64>) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("bench.smx");
    let base_ts = 1_700_000_000i64;
    // 500 pages * 512 bars/page = 256K bars
    let bars_per_flush = 512usize;
    let n_pages = 500usize;
    let mut s = Series::open(&path).unwrap();
    let mut keys = Vec::with_capacity(bars_per_flush * n_pages);
    for page in 0..n_pages {
        let chunk_base = base_ts + (page * bars_per_flush) as i64 * 900;
        let bars = make_bars(bars_per_flush, chunk_base);
        for bar in &bars {
            keys.push(xxh3_64(&bar.ts.to_le_bytes()));
        }
        s.append(&bars).unwrap();
    }
    s.flush_pending().unwrap();
    s.close().unwrap();
    (dir, path, keys)
}

fn bench_neg_lookup(c: &mut Criterion) {
    let (dir, path, keys) = build_500_page_corpus();
    let n_keys = keys.len();
    // Build filters from hashed keys
    let xf = SeriesXorFilter::build(&keys).unwrap();
    let mut bloom = Bloom::new();
    for &k in &keys {
        bloom.add(k);
    }

    // Negative probe keys (completely outside the time range)
    let base_ts = 1_700_000_000i64;
    let total_dur = n_keys as i64 * 900;
    let neg_start = base_ts + total_dur + 86400; // 1 day after series end
    let neg_end = neg_start + 3600;
    let neg_hashes: Vec<u64> = (0..1000)
        .map(|i| xxh3_64(&(neg_start + i * 900).to_le_bytes()))
        .collect();

    let mut group = c.benchmark_group("neg_lookup_500p");

    // Xor filter
    group.bench_function("xorf", |b| {
        b.iter(|| {
            let mut hit = false;
            for &h in &neg_hashes {
                hit |= xf.contains(h);
            }
            hit
        })
    });

    // Bloom filter
    group.bench_function("bloom_16kb", |b| {
        b.iter(|| {
            let mut hit = false;
            for &h in &neg_hashes {
                hit |= bloom.contains(h);
            }
            hit
        })
    });

    // No filter — simulated page-header scan (count pages outside range)
    let index_entries = 500usize;
    let page_ts_max = base_ts + total_dur;
    group.bench_function("no_filter", |b| {
        b.iter(|| {
            let mut scanned = 0usize;
            for i in 0..index_entries {
                let page_end = base_ts + (i as i64 + 1) * 512 * 900;
                if page_end >= neg_start && base_ts + i as i64 * 512 * 900 < neg_end {
                    scanned += 1;
                }
            }
            scanned
        })
    });

    // SQLite WAL
    let db_path = dir.path().join("bench.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE ts (t INTEGER PRIMARY KEY);")
            .unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        let mut stmt = conn.prepare("INSERT OR IGNORE INTO ts VALUES (?)").unwrap();
        for &k in &keys {
            stmt.execute(rusqlite::params![k as i64]).unwrap();
        }
        drop(stmt);
        tx.commit().unwrap();
    }
    group.bench_function("sqlite_wal", |b| {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        b.iter(|| {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM ts WHERE t BETWEEN ? AND ?",
                    rusqlite::params![neg_start, neg_end],
                    |r| r.get(0),
                )
                .unwrap();
            count
        })
    });

    group.finish();

    // Print xor size for caveman report
    eprintln!(
        "\n[xor_vs_bloom] n_keys={n_keys} xor_bytes={} bloom_bytes=16384",
        xf.size_bytes()
    );
}

criterion_group!(benches, bench_neg_lookup);
criterion_main!(benches);
