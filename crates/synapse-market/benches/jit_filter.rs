use criterion::{Criterion, black_box, criterion_group, criterion_main};
use synapse_market::jit::{Col, FilterCache, Op, Predicate};

const TICKERS: usize = 220;
const BARS: usize = 2880;
const N: usize = TICKERS * BARS;

fn make_data() -> (Vec<i64>, Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>) {
    let ts: Vec<i64> = (0..N as i64).collect();
    let open: Vec<f32> = (0..N).map(|i| 90.0 + (i % 50) as f32).collect();
    let high: Vec<f32> = open.iter().map(|v| v + 5.0).collect();
    let low: Vec<f32> = open.iter().map(|v| v - 5.0).collect();
    let close: Vec<f32> = (0..N).map(|i| 80.0 + (i % 60) as f32).collect();
    let volume: Vec<f32> = (0..N).map(|i| 5000.0 + (i % 20000) as f32).collect();
    (ts, open, high, low, close, volume)
}

fn bench_naive(c: &mut Criterion) {
    let (_, _, _, _, close, volume) = make_data();
    c.bench_function("naive_rust", |b| {
        b.iter(|| {
            let mut count = 0usize;
            for i in 0..N {
                if black_box(close[i]) > 100.0 && black_box(volume[i]) > 10000.0 {
                    count += 1;
                }
            }
            black_box(count)
        })
    });
}

fn bench_jit(c: &mut Criterion) {
    let (ts, open, high, low, close, volume) = make_data();
    let p = Predicate::And(
        Box::new(Predicate::Cmp(Col::Close, Op::Gt, 100.0)),
        Box::new(Predicate::Cmp(Col::Volume, Op::Gt, 10000.0)),
    );
    let mut cache = FilterCache::new();
    cache.get_or_compile(&p).unwrap(); // warm

    let mut mask = vec![0u8; N];
    c.bench_function("jit_filter", |b| {
        b.iter(|| {
            let compiled = cache.get_or_compile(&p).unwrap();
            let count = unsafe {
                (compiled.func_ptr)(
                    ts.as_ptr(),
                    open.as_ptr(),
                    high.as_ptr(),
                    low.as_ptr(),
                    close.as_ptr(),
                    volume.as_ptr(),
                    N,
                    mask.as_mut_ptr(),
                )
            };
            black_box(count)
        })
    });
}

fn bench_sqlite(c: &mut Criterion) {
    use rusqlite::Connection;
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE candles (close REAL, volume REAL);",
    )
    .unwrap();
    {
        let (_, _, _, _, close, volume) = make_data();
        let tx = conn.unchecked_transaction().unwrap();
        let mut stmt = conn.prepare("INSERT INTO candles VALUES (?1, ?2)").unwrap();
        for i in 0..N {
            stmt.execute(rusqlite::params![close[i], volume[i]])
                .unwrap();
        }
        tx.commit().unwrap();
    }

    c.bench_function("sqlite_wal", |b| {
        b.iter(|| {
            let count: i64 = conn
                .query_row(
                    "SELECT count(*) FROM candles WHERE close > 100 AND volume > 10000",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            black_box(count)
        })
    });
}

fn bench_duckdb(c: &mut Criterion) {
    use duckdb::Connection;
    let conn = Connection::open_in_memory().unwrap();
    let (_, _, _, _, close, volume) = make_data();

    conn.execute_batch("CREATE TABLE candles (close FLOAT, volume FLOAT)")
        .unwrap();
    {
        let mut app = conn.appender("candles").unwrap();
        for i in 0..N {
            app.append_row(duckdb::params![close[i] as f64, volume[i] as f64])
                .unwrap();
        }
        app.flush();
    }

    c.bench_function("duckdb_attach", |b| {
        b.iter(|| {
            let mut stmt = conn
                .prepare("SELECT count(*) FROM candles WHERE close > 100 AND volume > 10000")
                .unwrap();
            let count: i64 = stmt.query_row([], |r| r.get(0)).unwrap();
            black_box(count)
        })
    });
}

criterion_group!(benches, bench_naive, bench_jit, bench_sqlite, bench_duckdb);
criterion_main!(benches);
