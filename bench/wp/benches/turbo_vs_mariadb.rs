//! Turbo single-row INSERT vs MariaDB.
//!
//! TurboLibsqlStore = synchronous=OFF + WAL + mmap + cache + locking=EXCLUSIVE.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use mysql::prelude::*;
use mysql::{OptsBuilder, Pool};
use std::sync::Arc;
use synapse_libsql::Store;
use synapse_libsql::{LibsqlStore, TurboLibsqlStore};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().unwrap()
}

fn bench_turbo_insert(c: &mut Criterion) {
    let runtime = rt();
    let path = "/tmp/synapse-bench/turbo.db";
    std::fs::remove_file(path).ok();
    std::fs::remove_file(format!("{path}-wal")).ok();
    std::fs::remove_file(format!("{path}-shm")).ok();
    let store = runtime.block_on(async {
        let s = TurboLibsqlStore::open_local(path).await.unwrap();
        s.exec("CREATE TABLE bench (id INTEGER PRIMARY KEY AUTOINCREMENT, k TEXT, v TEXT)")
            .await
            .unwrap();
        Arc::new(s)
    });

    let mut counter = 0u64;
    c.bench_function("turbo_libsql_naive_insert", |b| {
        b.iter(|| {
            counter += 1;
            let sql = format!("INSERT INTO bench (k, v) VALUES ('k{counter}', 'v{counter}')");
            runtime.block_on(async {
                store.exec(black_box(&sql)).await.unwrap();
            });
        });
    });
}

fn bench_baseline_libsql(c: &mut Criterion) {
    let runtime = rt();
    let path = "/tmp/synapse-bench/baseline.db";
    std::fs::remove_file(path).ok();
    let store = runtime.block_on(async {
        let s = LibsqlStore::open_local(path).await.unwrap();
        s.exec("CREATE TABLE bench (id INTEGER PRIMARY KEY AUTOINCREMENT, k TEXT, v TEXT)")
            .await
            .unwrap();
        Arc::new(s)
    });

    let mut counter = 0u64;
    c.bench_function("baseline_libsql_no_pragma", |b| {
        b.iter(|| {
            counter += 1;
            let sql = format!("INSERT INTO bench (k, v) VALUES ('k{counter}', 'v{counter}')");
            runtime.block_on(async {
                store.exec(black_box(&sql)).await.unwrap();
            });
        });
    });
}

fn bench_mariadb_baseline(c: &mut Criterion) {
    let opts = OptsBuilder::new()
        .ip_or_hostname(Some("127.0.0.1"))
        .tcp_port(3307)
        .user(Some("root"))
        .db_name(Some("wp"));
    let pool = Pool::new(opts).expect("MariaDB pool");
    let mut conn = pool.get_conn().expect("conn");
    conn.query_drop("DROP TABLE IF EXISTS bench_t").ok();
    conn.query_drop(
        "CREATE TABLE bench_t (id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY, k VARCHAR(64), v TEXT) ENGINE=InnoDB",
    )
    .unwrap();

    let mut counter = 0u64;
    c.bench_function("mariadb_baseline", |b| {
        b.iter(|| {
            counter += 1;
            conn.exec_drop(
                "INSERT INTO bench_t (k, v) VALUES (?, ?)",
                (format!("k{counter}"), format!("v{counter}")),
            )
            .unwrap();
        });
    });
}

criterion_group!(benches, bench_baseline_libsql, bench_turbo_insert, bench_mariadb_baseline);
criterion_main!(benches);
