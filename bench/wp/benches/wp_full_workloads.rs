//! Real WP workloads: posts listing + insert throughput.
//!
//! MariaDB vs synapse cache vs synapse in-process libsql.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use mysql::prelude::*;
use mysql::{OptsBuilder, Pool};

fn maria_pool() -> Pool {
    let opts = OptsBuilder::new()
        .ip_or_hostname(Some("127.0.0.1"))
        .tcp_port(3307)
        .user(Some("root"))
        .db_name(Some("wp"));
    Pool::new(opts).expect("MariaDB pool")
}

// 1. Posts listing — typical WP_Query (homepage)
fn bench_posts_listing(c: &mut Criterion) {
    let pool = maria_pool();
    let mut conn = pool.get_conn().expect("conn");

    c.bench_function("mariadb_posts_listing_10", |b| {
        b.iter(|| {
            let rows: Vec<(u64, String, String)> = conn
                .query(
                    "SELECT ID, post_title, post_status FROM wp_posts WHERE post_type='post' AND post_status='publish' ORDER BY post_date DESC LIMIT 10",
                )
                .unwrap();
            black_box(rows.len());
        });
    });

    c.bench_function("mariadb_posts_count", |b| {
        b.iter(|| {
            let n: Option<u64> = conn
                .query_first("SELECT COUNT(*) FROM wp_posts WHERE post_status='publish'")
                .unwrap();
            black_box(n);
        });
    });
}

// 2. Insert throughput — comment posting / option update
fn bench_inserts(c: &mut Criterion) {
    let pool = maria_pool();
    let mut conn = pool.get_conn().expect("conn");
    conn.query_drop("DROP TABLE IF EXISTS bench_inserts").ok();
    conn.query_drop(
        "CREATE TABLE bench_inserts (id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY, k VARCHAR(64), v TEXT) ENGINE=InnoDB",
    )
    .unwrap();

    let mut counter = 0u64;
    c.bench_function("mariadb_insert_single", |b| {
        b.iter(|| {
            counter += 1;
            conn.exec_drop(
                "INSERT INTO bench_inserts (k, v) VALUES (?, ?)",
                (format!("k{counter}"), format!("v{counter}")),
            )
            .unwrap();
        });
    });
}

criterion_group!(benches, bench_posts_listing, bench_inserts);
criterion_main!(benches);
