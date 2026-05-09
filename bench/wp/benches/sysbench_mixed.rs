//! Sysbench-style mixed workload: 80% reads, 20% writes, 8 threads.
//! Models real OLTP traffic.
//!
//! Warmup: 10k rows pre-inserted.
//! Each iteration: 5 reads + 1 write per task × 8 tasks = 48 ops/iter.

use criterion::{criterion_group, criterion_main, Criterion};
use mysql::prelude::*;
use mysql::{OptsBuilder, Pool};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use synapse_libsql::Store;
use synapse_libsql::{PoolTurboLibsqlStore, RealPoolStore};

const TASKS: usize = 8;
const READS_PER_TASK: usize = 5;
const WRITES_PER_TASK: usize = 1;
const WARMUP_ROWS: usize = 10_000;

fn now_id() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64
}

fn bench_pool_turbo_mixed(c: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(8)
        .enable_all()
        .build()
        .unwrap();
    c.bench_function("pool_turbo_8t_mixed_80r20w", |b| {
        b.iter_custom(|iters| {
            let elapsed = runtime.block_on(async {
                let path = format!("/tmp/synapse-bench/sysbench_{}.db", now_id());
                std::fs::remove_file(&path).ok();
                std::fs::remove_file(format!("{path}-wal")).ok();
                let store = Arc::new(PoolTurboLibsqlStore::open_local(&path).await.unwrap());
                store.exec("CREATE TABLE t (id INTEGER PRIMARY KEY AUTOINCREMENT, k TEXT, v TEXT)").await.unwrap();
                // Warmup
                for i in 0..WARMUP_ROWS {
                    store.exec(&format!("INSERT INTO t (k, v) VALUES ('k{i}', 'v{i}')")).await.unwrap();
                }
                let counter = Arc::new(AtomicU64::new(WARMUP_ROWS as u64));
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    let mut handles = vec![];
                    for tid in 0..TASKS {
                        let s = store.clone();
                        let c = counter.clone();
                        handles.push(tokio::spawn(async move {
                            // 5 reads
                            for _ in 0..READS_PER_TASK {
                                let id = (now_id() % WARMUP_ROWS as u64).max(1);
                                let _ = s.query(&format!("SELECT v FROM t WHERE id = {id}")).await;
                            }
                            // 1 write
                            for _ in 0..WRITES_PER_TASK {
                                let n = c.fetch_add(1, Ordering::Relaxed);
                                let _ = s.exec(&format!("INSERT INTO t (k, v) VALUES ('t{tid}_k{n}', 'v{n}')")).await;
                            }
                        }));
                    }
                    for h in handles {
                        let _ = h.await;
                    }
                }
                start.elapsed()
            });
            elapsed
        });
    });
}

fn bench_mariadb_mixed(c: &mut Criterion) {
    c.bench_function("mariadb_8t_mixed_80r20w", |b| {
        b.iter_custom(|iters| {
            let opts = OptsBuilder::new()
                .ip_or_hostname(Some("127.0.0.1"))
                .tcp_port(3307)
                .user(Some("root"))
                .db_name(Some("wp"));
            let pool = Pool::new(opts).expect("pool");
            {
                let mut conn = pool.get_conn().unwrap();
                conn.query_drop("DROP TABLE IF EXISTS sysbench_t").ok();
                conn.query_drop("CREATE TABLE sysbench_t (id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY, k VARCHAR(64), v TEXT) ENGINE=InnoDB").unwrap();
                for i in 0..WARMUP_ROWS {
                    conn.exec_drop(
                        "INSERT INTO sysbench_t (k, v) VALUES (?, ?)",
                        (format!("k{i}"), format!("v{i}")),
                    ).unwrap();
                }
            }
            let counter = Arc::new(AtomicU64::new(WARMUP_ROWS as u64));
            let start = std::time::Instant::now();
            for _ in 0..iters {
                let mut handles = vec![];
                for tid in 0..TASKS {
                    let p = pool.clone();
                    let c = counter.clone();
                    handles.push(std::thread::spawn(move || {
                        let mut conn = p.get_conn().unwrap();
                        for _ in 0..READS_PER_TASK {
                            let id = (now_id() % WARMUP_ROWS as u64).max(1);
                            let _: Option<String> = conn.exec_first(
                                "SELECT v FROM sysbench_t WHERE id = ?",
                                (id,),
                            ).ok().flatten();
                        }
                        for _ in 0..WRITES_PER_TASK {
                            let n = c.fetch_add(1, Ordering::Relaxed);
                            conn.exec_drop(
                                "INSERT INTO sysbench_t (k, v) VALUES (?, ?)",
                                (format!("t{tid}_k{n}"), format!("v{n}")),
                            ).ok();
                        }
                    }));
                }
                for h in handles {
                    h.join().unwrap();
                }
            }
            start.elapsed()
        });
    });
}

fn bench_real_pool_mixed(c: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(8)
        .enable_all()
        .build()
        .unwrap();
    c.bench_function("real_pool_8t_mixed_80r20w", |b| {
        b.iter_custom(|iters| {
            let elapsed = runtime.block_on(async {
                let path = format!("/tmp/synapse-bench/realpool_{}.db", now_id());
                std::fs::remove_file(&path).ok();
                std::fs::remove_file(format!("{path}-wal")).ok();
                let store = Arc::new(RealPoolStore::open_local(&path, 8).await.unwrap());
                store.exec("CREATE TABLE t (id INTEGER PRIMARY KEY AUTOINCREMENT, k TEXT, v TEXT)").await.unwrap();
                for i in 0..WARMUP_ROWS {
                    store.exec(&format!("INSERT INTO t (k, v) VALUES ('k{i}', 'v{i}')")).await.unwrap();
                }
                let counter = Arc::new(AtomicU64::new(WARMUP_ROWS as u64));
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    let mut handles = vec![];
                    for tid in 0..TASKS {
                        let s = store.clone();
                        let c = counter.clone();
                        handles.push(tokio::spawn(async move {
                            for _ in 0..READS_PER_TASK {
                                let id = (now_id() % WARMUP_ROWS as u64).max(1);
                                let _ = s.query(&format!("SELECT v FROM t WHERE id = {id}")).await;
                            }
                            for _ in 0..WRITES_PER_TASK {
                                let n = c.fetch_add(1, Ordering::Relaxed);
                                let _ = s.exec(&format!("INSERT INTO t (k, v) VALUES ('t{tid}_k{n}', 'v{n}')")).await;
                            }
                        }));
                    }
                    for h in handles {
                        let _ = h.await;
                    }
                }
                start.elapsed()
            });
            elapsed
        });
    });
}

criterion_group!(
    name = benches;
    config = Criterion::default().sample_size(10).measurement_time(std::time::Duration::from_secs(15));
    targets = bench_real_pool_mixed, bench_pool_turbo_mixed, bench_mariadb_mixed
);
criterion_main!(benches);
