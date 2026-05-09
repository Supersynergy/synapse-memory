//! Group-commit batched INSERT vs MariaDB single INSERT.
//!
//! Goal: close the 12× INSERT gap → flip to 5-100× advantage via batching.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use mysql::prelude::*;
use mysql::{OptsBuilder, Pool};
use std::sync::Arc;
use synapse_libsql::Store;
use synapse_libsql::{BatchedLibsqlStore, LibsqlStore};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().unwrap()
}

fn bench_batched_libsql(c: &mut Criterion) {
    let runtime = rt();
    for batch in [10usize, 100, 1000] {
        let path = format!("/tmp/synapse-bench/batch-{batch}.db");
        std::fs::remove_file(&path).ok();
        let store = runtime.block_on(async {
            let s = BatchedLibsqlStore::open_local(&path, batch).await.unwrap();
            s.exec("CREATE TABLE bench (id INTEGER PRIMARY KEY AUTOINCREMENT, k TEXT, v TEXT)")
                .await
                .unwrap();
            s.flush().await.unwrap();
            Arc::new(s)
        });

        let mut counter = 0u64;
        c.bench_function(&format!("libsql_batched_b{batch}_amortized"), |b| {
            b.iter(|| {
                counter += 1;
                let sql = format!("INSERT INTO bench (k, v) VALUES ('k{counter}', 'v{counter}')");
                runtime.block_on(async {
                    store.exec(black_box(&sql)).await.unwrap();
                });
            });
        });
        // Final flush
        runtime.block_on(async {
            store.flush().await.unwrap();
        });
    }
}

fn bench_naive_libsql(c: &mut Criterion) {
    let runtime = rt();
    let path = "/tmp/synapse-bench/naive.db";
    std::fs::remove_file(path).ok();
    let store = runtime.block_on(async {
        let s = LibsqlStore::open_local(path).await.unwrap();
        s.exec("CREATE TABLE bench (id INTEGER PRIMARY KEY AUTOINCREMENT, k TEXT, v TEXT)")
            .await
            .unwrap();
        Arc::new(s)
    });

    let mut counter = 0u64;
    c.bench_function("libsql_naive_per_row_fsync", |b| {
        b.iter(|| {
            counter += 1;
            let sql = format!("INSERT INTO bench (k, v) VALUES ('k{counter}', 'v{counter}')");
            runtime.block_on(async {
                store.exec(black_box(&sql)).await.unwrap();
            });
        });
    });
}

fn bench_mariadb(c: &mut Criterion) {
    let opts = OptsBuilder::new()
        .ip_or_hostname(Some("127.0.0.1"))
        .tcp_port(3307)
        .user(Some("root"))
        .db_name(Some("wp"));
    let pool = Pool::new(opts).expect("MariaDB pool");
    let mut conn = pool.get_conn().expect("conn");
    conn.query_drop("DROP TABLE IF EXISTS bench_b").ok();
    conn.query_drop(
        "CREATE TABLE bench_b (id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY, k VARCHAR(64), v TEXT) ENGINE=InnoDB",
    )
    .unwrap();

    let mut counter = 0u64;
    c.bench_function("mariadb_naive_per_row", |b| {
        b.iter(|| {
            counter += 1;
            conn.exec_drop(
                "INSERT INTO bench_b (k, v) VALUES (?, ?)",
                (format!("k{counter}"), format!("v{counter}")),
            )
            .unwrap();
        });
    });
}

criterion_group!(benches, bench_naive_libsql, bench_batched_libsql, bench_mariadb);
criterion_main!(benches);
