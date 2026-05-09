//! libsql (synapse row backend) vs MariaDB on identical WP workloads.
//!
//! Tests whether embedded libsql beats wire-MariaDB even for raw SELECT/INSERT.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use mysql::prelude::*;
use mysql::{OptsBuilder, Pool};
use std::sync::Arc;
use synapse_libsql::Store;
use synapse_libsql::LibsqlStore;

fn maria_pool() -> Pool {
    let opts = OptsBuilder::new()
        .ip_or_hostname(Some("127.0.0.1"))
        .tcp_port(3307)
        .user(Some("root"))
        .db_name(Some("wp"));
    Pool::new(opts).expect("MariaDB pool")
}

fn setup_libsql() -> Arc<LibsqlStore> {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let store = rt.block_on(async {
        let path = "/tmp/synapse-bench/libsql-bench.db";
        std::fs::remove_file(path).ok();
        let s = LibsqlStore::open_local(path).await.unwrap();
        s.exec("CREATE TABLE wp_options (option_name TEXT PRIMARY KEY, option_value TEXT, autoload TEXT DEFAULT 'yes')").await.unwrap();
        for i in 0..2000 {
            s.exec(&format!(
                "INSERT INTO wp_options (option_name, option_value) VALUES ('opt_{i}', 'value_{i}')"
            )).await.unwrap();
        }
        Arc::new(s)
    });
    // Leak the runtime to keep it alive
    Box::leak(Box::new(rt));
    store
}

fn bench_libsql_inserts(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let store = rt.block_on(async {
        let path = "/tmp/synapse-bench/libsql-insert-bench.db";
        std::fs::remove_file(path).ok();
        let s = LibsqlStore::open_local(path).await.unwrap();
        s.exec("CREATE TABLE bench (id INTEGER PRIMARY KEY AUTOINCREMENT, k TEXT, v TEXT)")
            .await
            .unwrap();
        s
    });

    let mut counter = 0u64;
    c.bench_function("libsql_insert_single", |b| {
        b.iter(|| {
            counter += 1;
            let sql = format!("INSERT INTO bench (k, v) VALUES ('k{counter}', 'v{counter}')");
            rt.block_on(async {
                store.exec(&sql).await.unwrap();
            });
        });
    });
}

fn bench_libsql_select(c: &mut Criterion) {
    let store = setup_libsql();
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("libsql_options_count", |b| {
        b.iter(|| {
            let v = rt.block_on(async {
                store.exec("SELECT COUNT(*) FROM wp_options").await.unwrap()
            });
            black_box(v);
        });
    });
}

fn bench_mariadb_baselines(c: &mut Criterion) {
    let pool = maria_pool();
    let mut conn = pool.get_conn().expect("conn");

    let mut counter = 100000u64;
    c.bench_function("mariadb_insert_baseline", |b| {
        b.iter(|| {
            counter += 1;
            conn.exec_drop(
                "INSERT INTO bench_inserts (k, v) VALUES (?, ?)",
                (format!("z{counter}"), format!("z{counter}")),
            )
            .unwrap();
        });
    });

    c.bench_function("mariadb_options_count_baseline", |b| {
        b.iter(|| {
            let n: Option<u64> = conn
                .query_first("SELECT COUNT(*) FROM wp_options")
                .unwrap();
            black_box(n);
        });
    });
}

criterion_group!(benches, bench_libsql_inserts, bench_libsql_select, bench_mariadb_baselines);
criterion_main!(benches);
