//! Concurrent load bench: 8 tokio tasks hammering INSERT simultaneously.
//!
//! This is the "real production" test — single-thread bench is for ceiling,
//! concurrent bench is for actual sustained throughput.

use criterion::{criterion_group, criterion_main, Criterion};
use mysql::prelude::*;
use mysql::{OptsBuilder, Pool};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use synapse_libsql::Store;
use synapse_libsql::{TurboLibsqlStore, BatchedLibsqlStore};

const TASKS: usize = 8;
const ROWS_PER_TASK: usize = 1000;

fn bench_turbo_concurrent(c: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(8)
        .enable_all()
        .build()
        .unwrap();
    c.bench_function("turbo_8t_8000_inserts", |b| {
        b.iter_custom(|iters| {
            let elapsed = runtime.block_on(async {
                let mut total = std::time::Duration::ZERO;
                for _ in 0..iters {
                    let path = format!("/tmp/synapse-bench/turbo_conc_{}.db", uuid_like());
                    std::fs::remove_file(&path).ok();
                    std::fs::remove_file(format!("{path}-wal")).ok();
                    let store = Arc::new(TurboLibsqlStore::open_local(&path).await.unwrap());
                    store.exec("CREATE TABLE t (id INTEGER PRIMARY KEY AUTOINCREMENT, k TEXT, v TEXT)").await.unwrap();

                    let counter = Arc::new(AtomicU64::new(0));
                    let start = std::time::Instant::now();
                    let mut handles = vec![];
                    for tid in 0..TASKS {
                        let s = store.clone();
                        let c = counter.clone();
                        handles.push(tokio::spawn(async move {
                            for _ in 0..ROWS_PER_TASK {
                                let n = c.fetch_add(1, Ordering::Relaxed);
                                let sql = format!("INSERT INTO t (k, v) VALUES ('t{tid}_{n}', 'v{n}')");
                                let _ = s.exec(&sql).await;
                            }
                        }));
                    }
                    for h in handles {
                        let _ = h.await;
                    }
                    total += start.elapsed();
                }
                total
            });
            elapsed
        });
    });
}

fn bench_batched_concurrent(c: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(8)
        .enable_all()
        .build()
        .unwrap();
    c.bench_function("batched_b100_8t_8000_inserts", |b| {
        b.iter_custom(|iters| {
            let elapsed = runtime.block_on(async {
                let mut total = std::time::Duration::ZERO;
                for _ in 0..iters {
                    let path = format!("/tmp/synapse-bench/batched_conc_{}.db", uuid_like());
                    std::fs::remove_file(&path).ok();
                    std::fs::remove_file(format!("{path}-wal")).ok();
                    let store = Arc::new(BatchedLibsqlStore::open_local(&path, 100).await.unwrap());
                    store.exec("CREATE TABLE t (id INTEGER PRIMARY KEY AUTOINCREMENT, k TEXT, v TEXT)").await.unwrap();
                    store.flush().await.unwrap();

                    let counter = Arc::new(AtomicU64::new(0));
                    let start = std::time::Instant::now();
                    let mut handles = vec![];
                    for tid in 0..TASKS {
                        let s = store.clone();
                        let c = counter.clone();
                        handles.push(tokio::spawn(async move {
                            for _ in 0..ROWS_PER_TASK {
                                let n = c.fetch_add(1, Ordering::Relaxed);
                                let sql = format!("INSERT INTO t (k, v) VALUES ('t{tid}_{n}', 'v{n}')");
                                let _ = s.exec(&sql).await;
                            }
                        }));
                    }
                    for h in handles {
                        let _ = h.await;
                    }
                    store.flush().await.unwrap();
                    total += start.elapsed();
                }
                total
            });
            elapsed
        });
    });
}

fn bench_mariadb_concurrent(c: &mut Criterion) {
    c.bench_function("mariadb_8t_8000_inserts", |b| {
        b.iter_custom(|iters| {
            let mut total = std::time::Duration::ZERO;
            for _ in 0..iters {
                let opts = OptsBuilder::new()
                    .ip_or_hostname(Some("127.0.0.1"))
                    .tcp_port(3307)
                    .user(Some("root"))
                    .db_name(Some("wp"));
                let pool = Pool::new(opts).expect("pool");
                {
                    let mut conn = pool.get_conn().unwrap();
                    conn.query_drop("DROP TABLE IF EXISTS bench_c").ok();
                    conn.query_drop("CREATE TABLE bench_c (id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY, k VARCHAR(64), v TEXT) ENGINE=InnoDB").unwrap();
                }

                let counter = Arc::new(AtomicU64::new(0));
                let start = std::time::Instant::now();
                let mut handles = vec![];
                for tid in 0..TASKS {
                    let p = pool.clone();
                    let c = counter.clone();
                    handles.push(std::thread::spawn(move || {
                        let mut conn = p.get_conn().unwrap();
                        for _ in 0..ROWS_PER_TASK {
                            let n = c.fetch_add(1, Ordering::Relaxed);
                            conn.exec_drop(
                                "INSERT INTO bench_c (k, v) VALUES (?, ?)",
                                (format!("t{tid}_{n}"), format!("v{n}")),
                            ).ok();
                        }
                    }));
                }
                for h in handles {
                    h.join().unwrap();
                }
                total += start.elapsed();
            }
            total
        });
    });
}

fn uuid_like() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64
}

criterion_group!(
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = bench_turbo_concurrent, bench_batched_concurrent, bench_mariadb_concurrent
);
criterion_main!(benches);
