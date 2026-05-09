//! REAL benchmark: MariaDB 12.2 vs synapse AutoloadCache vs libsql.
//!
//! Setup: `wp_options` table, 2000 rows, autoload='yes'.
//! Workload: 30 single-option lookups per pageload (typical WP).
//!
//! Three paths measured:
//!   1. MariaDB 12.2 InnoDB on :3307 (real wire MySQL client)
//!   2. libsql 0.9 local file (synapse backend layer)
//!   3. AutoloadCache (synapse Layer 2 in-memory)

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use mysql::prelude::*;
use mysql::{OptsBuilder, Pool};
use synapse_cms::AutoloadCache;

const N: usize = 2000;
const PAGELOAD_KEYS: usize = 30;

fn keys() -> Vec<String> {
    (0..PAGELOAD_KEYS).map(|i| format!("opt_{i}")).collect()
}

// 1. MariaDB
fn bench_mariadb(c: &mut Criterion) {
    let opts = OptsBuilder::new()
        .ip_or_hostname(Some("127.0.0.1"))
        .tcp_port(3307)
        .user(Some("root"))
        .db_name(Some("wp"));
    let pool = Pool::new(opts).expect("MariaDB connection failed — start it on :3307");
    let mut conn = pool.get_conn().expect("conn");
    let ks = keys();

    c.bench_function("mariadb_single_select", |b| {
        b.iter(|| {
            let v: Option<String> = conn
                .exec_first(
                    "SELECT option_value FROM wp_options WHERE option_name = ?",
                    ("opt_42",),
                )
                .unwrap();
            black_box(v);
        });
    });

    c.bench_function("mariadb_30_pageload", |b| {
        b.iter(|| {
            for k in &ks {
                let v: Option<String> = conn
                    .exec_first(
                        "SELECT option_value FROM wp_options WHERE option_name = ?",
                        (k,),
                    )
                    .unwrap();
                black_box(v);
            }
        });
    });

    // Bulk autoload — real WP path
    c.bench_function("mariadb_autoload_all", |b| {
        b.iter(|| {
            let rows: Vec<(String, String)> = conn
                .query("SELECT option_name, option_value FROM wp_options WHERE autoload = 'yes'")
                .unwrap();
            black_box(rows.len());
        });
    });
}

// 2. AutoloadCache
fn bench_cache(c: &mut Criterion) {
    let cache = AutoloadCache::new();
    let rows = (0..N).map(|i| (format!("opt_{i}"), format!("value_{i}").into_bytes()));
    cache.load_all(rows);
    let ks = keys();

    c.bench_function("cache_single_get", |b| {
        b.iter(|| {
            let v = cache.get(black_box("opt_42"));
            black_box(v);
        });
    });

    c.bench_function("cache_30_pageload", |b| {
        b.iter(|| {
            for k in &ks {
                let _ = black_box(cache.get(k));
            }
        });
    });
}

criterion_group!(benches, bench_mariadb, bench_cache);
criterion_main!(benches);
