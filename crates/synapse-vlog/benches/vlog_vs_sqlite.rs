use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use rusqlite::Connection;
use synapse_vlog::VlogStore;
use tempfile::tempdir;

const N: u64 = 1_000_000;
const VAL: &[u8] = b"value_16bytes___"; // 16 bytes

fn bench_vlog_put(c: &mut Criterion) {
    let mut g = c.benchmark_group("put_1M");
    g.throughput(Throughput::Elements(N));
    g.sample_size(10);

    g.bench_function("vlog", |b| {
        b.iter(|| {
            let dir = tempdir().unwrap();
            let mut store = VlogStore::open(dir.path()).unwrap();
            for i in 0u64..N {
                store.put(i.to_le_bytes().to_vec(), VAL).unwrap();
            }
            store.flush().unwrap();
        })
    });

    g.bench_function("sqlite", |b| {
        b.iter(|| {
            let dir = tempdir().unwrap();
            let conn = Connection::open(dir.path().join("bench.db")).unwrap();
            conn.execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
                 CREATE TABLE kv (k BLOB PRIMARY KEY, v BLOB);",
            )
            .unwrap();
            let mut stmt = conn
                .prepare("INSERT OR REPLACE INTO kv VALUES (?,?)")
                .unwrap();
            let tx = conn.unchecked_transaction().unwrap();
            for i in 0u64..N {
                stmt.execute(rusqlite::params![i.to_le_bytes().as_ref(), VAL])
                    .unwrap();
            }
            tx.commit().unwrap();
        })
    });

    g.finish();
}

fn bench_vlog_get(c: &mut Criterion) {
    let mut g = c.benchmark_group("get_1M");
    g.throughput(Throughput::Elements(N));
    g.sample_size(10);

    g.bench_function("vlog", |b| {
        let dir = tempdir().unwrap();
        let mut store = VlogStore::open(dir.path()).unwrap();
        for i in 0u64..N {
            store.put(i.to_le_bytes().to_vec(), VAL).unwrap();
        }
        store.flush().unwrap();
        b.iter(|| {
            for i in 0u64..N {
                let _ = store.get(&i.to_le_bytes().to_vec()).unwrap();
            }
        })
    });

    g.bench_function("sqlite", |b| {
        let dir = tempdir().unwrap();
        let conn = Connection::open(dir.path().join("bench.db")).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
             CREATE TABLE kv (k BLOB PRIMARY KEY, v BLOB);",
        )
        .unwrap();
        {
            let mut stmt = conn
                .prepare("INSERT OR REPLACE INTO kv VALUES (?,?)")
                .unwrap();
            let tx = conn.unchecked_transaction().unwrap();
            for i in 0u64..N {
                stmt.execute(rusqlite::params![i.to_le_bytes().as_ref(), VAL])
                    .unwrap();
            }
            tx.commit().unwrap();
        }
        let mut stmt = conn.prepare("SELECT v FROM kv WHERE k=?").unwrap();
        b.iter(|| {
            for i in 0u64..N {
                let _: Vec<u8> = stmt
                    .query_row(rusqlite::params![i.to_le_bytes().as_ref()], |r| r.get(0))
                    .unwrap();
            }
        })
    });

    g.finish();
}

criterion_group!(benches, bench_vlog_put, bench_vlog_get);
criterion_main!(benches);
