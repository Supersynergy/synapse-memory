/// Cross-language ABI differential test.
///
/// Rust-only: full differential vs SQLite + DuckDB (blake3 hash of result JSON).
/// Python/TS stubs: see tests/py_query.py and tests/ts_query.ts — no pyo3/bun-ffi present,
/// so cross-lang ABI is Rust-only for now.
use duckdb::Connection as DuckConn;
use rusqlite::Connection as SqliteConn;
use tempfile::TempDir;

use synapse_market::series::Series;
use synapse_market::store::page::Bar;

struct Lcg(u64);
impl Lcg {
    fn new(seed: u64) -> Self { Self(seed) }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0
    }
    fn next_f64(&mut self) -> f64 { (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64 }
}

fn make_fixture(n: usize, seed: u64) -> Vec<Bar> {
    let mut rng = Lcg::new(seed);
    let mut close = 50.0f32;
    (0..n).map(|i| {
        let ret = (rng.next_f64() as f32 - 0.5) * 0.06;
        close = (close * (1.0 + ret)).max(1.0);
        Bar {
            ts: 1_700_000_000i64 + i as i64 * 3600,
            open: close * (1.0 - rng.next_f64() as f32 * 0.005),
            high: close * (1.0 + rng.next_f64() as f32 * 0.01),
            low:  close * (1.0 - rng.next_f64() as f32 * 0.01),
            close,
            volume: 10_000.0 * (0.5 + rng.next_f64() as f32),
        }
    }).collect()
}

fn bars_to_json(bars: &[Bar]) -> String {
    let rows: Vec<String> = bars.iter().map(|b| {
        format!(r#"{{"ts":{},"open":{:.4},"high":{:.4},"low":{:.4},"close":{:.4},"volume":{:.2}}}"#,
            b.ts, b.open, b.high, b.low, b.close, b.volume)
    }).collect();
    format!("[{}]", rows.join(","))
}

fn hash_json(json: &str) -> String {
    let hash = blake3::hash(json.as_bytes());
    hash.to_hex().to_string()
}

#[test]
fn abi_rust_only_hash_agreement() {
    let dir = TempDir::new().unwrap();
    let bars = make_fixture(2048, 0xF00D_CAFE);
    let ticker = "ABI_TEST";

    // Synapse-X
    let path = dir.path().join("abi.smx");
    let mut s = Series::open(&path).unwrap();
    s.append(&bars).unwrap();
    s.close().unwrap();
    let mut s = Series::open(&path).unwrap();

    // SQLite
    let sq = SqliteConn::open_in_memory().unwrap();
    sq.execute_batch(
        "CREATE TABLE ohlcv (ts INTEGER PRIMARY KEY, open REAL, high REAL, low REAL, close REAL, volume REAL);"
    ).unwrap();
    {
        let mut stmt = sq.prepare("INSERT INTO ohlcv VALUES(?,?,?,?,?,?)").unwrap();
        for b in &bars {
            stmt.execute(rusqlite::params![b.ts, b.open as f64, b.high as f64, b.low as f64, b.close as f64, b.volume as f64]).unwrap();
        }
    }

    // DuckDB
    let dk = DuckConn::open_in_memory().unwrap();
    dk.execute_batch(
        "CREATE TABLE ohlcv (ts BIGINT, open FLOAT, high FLOAT, low FLOAT, close FLOAT, volume FLOAT);"
    ).unwrap();
    {
        let mut app = dk.appender("ohlcv").unwrap();
        for b in &bars {
            app.append_row(duckdb::params![b.ts, b.open as f64, b.high as f64, b.low as f64, b.close as f64, b.volume as f64]).unwrap();
        }
        app.flush().unwrap();
    }

    // Query: middle 512 bars
    let mid_start = bars[256].ts;
    let mid_end   = bars[768].ts;

    let smx_bars = s.range(mid_start..mid_end).unwrap();

    let sq_bars: Vec<Bar> = {
        let mut stmt = sq.prepare(
            "SELECT ts,open,high,low,close,volume FROM ohlcv WHERE ts>=? AND ts<? ORDER BY ts"
        ).unwrap();
        stmt.query_map(rusqlite::params![mid_start, mid_end], |r| Ok(Bar {
            ts:     r.get::<_,i64>(0)?,
            open:   r.get::<_,f64>(1)? as f32,
            high:   r.get::<_,f64>(2)? as f32,
            low:    r.get::<_,f64>(3)? as f32,
            close:  r.get::<_,f64>(4)? as f32,
            volume: r.get::<_,f64>(5)? as f32,
        })).unwrap().map(|x| x.unwrap()).collect()
    };

    let dk_bars: Vec<Bar> = {
        let mut stmt = dk.prepare(
            "SELECT ts,open,high,low,close,volume FROM ohlcv WHERE ts>=? AND ts<? ORDER BY ts"
        ).unwrap();
        stmt.query_map(duckdb::params![mid_start, mid_end], |r| Ok(Bar {
            ts:     r.get::<_,i64>(0)?,
            open:   r.get::<_,f64>(1)? as f32,
            high:   r.get::<_,f64>(2)? as f32,
            low:    r.get::<_,f64>(3)? as f32,
            close:  r.get::<_,f64>(4)? as f32,
            volume: r.get::<_,f64>(5)? as f32,
        })).unwrap().map(|x| x.unwrap()).collect()
    };

    let _ = ticker; // docs reference only

    let smx_json = bars_to_json(&smx_bars);
    let sq_json  = bars_to_json(&sq_bars);
    let dk_json  = bars_to_json(&dk_bars);

    let smx_hash = hash_json(&smx_json);
    let sq_hash  = hash_json(&sq_json);
    let dk_hash  = hash_json(&dk_json);

    assert_eq!(smx_bars.len(), sq_bars.len(),
        "row count: smx={} sqlite={}", smx_bars.len(), sq_bars.len());
    assert_eq!(smx_bars.len(), dk_bars.len(),
        "row count: smx={} duckdb={}", smx_bars.len(), dk_bars.len());
    assert_eq!(smx_hash, sq_hash,
        "hash mismatch smx vs sqlite\nsmx:    {smx_hash}\nsqlite: {sq_hash}");
    assert_eq!(smx_hash, dk_hash,
        "hash mismatch smx vs duckdb\nsmx:    {smx_hash}\nduckdb: {dk_hash}");

    eprintln!("ABI blake3 hash: {smx_hash} (all 3 impls agree)");
}

/// Documents what py_query.py and ts_query.ts would do once pyo3/bun-ffi bindings exist.
#[test]
fn abi_cross_lang_stub_documented() {
    // Status: Rust-only differential complete (blake3 hash agreement above).
    // Python stub: tests/py_query.py — needs `pip install synapse-market` pyo3 wheel.
    // TS stub:     tests/ts_query.ts — needs bun-ffi + synapse_market.dylib.
    // Both stubs are in-tree but not executable without bindings.
    // Re-enable when pyo3 feature gate `synapse-market/python` is added.
    eprintln!("cross-lang ABI status: rust-only (python/TS stubs in tests/ — see py_query.py, ts_query.ts)");
}
