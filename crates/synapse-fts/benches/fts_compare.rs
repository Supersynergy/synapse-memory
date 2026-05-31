use criterion::{Criterion, black_box, criterion_group, criterion_main};
use rusqlite::Connection;
use std::path::Path;
use synapse_fts::FtsIndex;
use tempfile::TempDir;

const N_DOCS: usize = 10_000;
const N_QUERIES: usize = 100;

static QUERIES: &[&str] = &[
    "search engine performance",
    "rust programming language",
    "vector similarity search",
    "machine learning model",
    "database index structure",
    "information retrieval system",
    "natural language processing",
    "benchmark comparison result",
    "document ranking algorithm",
    "full text search query",
];

fn make_doc(i: usize) -> String {
    format!(
        "document {} about {} and {} with some extra tokens like {} {} {}",
        i,
        QUERIES[i % QUERIES.len()],
        QUERIES[(i + 3) % QUERIES.len()],
        i * 7,
        i * 13,
        i * 17
    )
}

fn bench_tantivy(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let mut idx = FtsIndex::new(dir.path()).unwrap();
    for i in 0..N_DOCS {
        idx.add(i as u64, &make_doc(i)).unwrap();
    }
    idx.commit().unwrap();

    let queries: Vec<String> = (0..N_QUERIES)
        .map(|i| QUERIES[i % QUERIES.len()].to_string())
        .collect();

    c.bench_function("tantivy_search_10k_docs_100q", |b| {
        b.iter(|| {
            for q in &queries {
                let r = idx.search(black_box(q), 10).unwrap();
                black_box(r);
            }
        })
    });
}

fn bench_sqlite_fts5(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("fts5.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute_batch(
        "CREATE VIRTUAL TABLE docs USING fts5(doc_id UNINDEXED, text, tokenize='porter unicode61');",
    )
    .unwrap();
    {
        let mut stmt = conn
            .prepare("INSERT INTO docs(doc_id, text) VALUES (?1, ?2)")
            .unwrap();
        for i in 0..N_DOCS {
            stmt.execute(rusqlite::params![i as i64, make_doc(i)])
                .unwrap();
        }
    }

    let queries: Vec<String> = (0..N_QUERIES)
        .map(|i| QUERIES[i % QUERIES.len()].to_string())
        .collect();

    c.bench_function("sqlite_fts5_search_10k_docs_100q", |b| {
        b.iter(|| {
            for q in &queries {
                let mut stmt = conn
                    .prepare_cached(
                        "SELECT doc_id, rank FROM docs WHERE text MATCH ?1 ORDER BY rank LIMIT 10",
                    )
                    .unwrap();
                let rows: Vec<(i64, f64)> = stmt
                    .query_map(rusqlite::params![black_box(q)], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })
                    .unwrap()
                    .map(|r| r.unwrap())
                    .collect();
                black_box(rows);
            }
        })
    });
}

criterion_group!(benches, bench_tantivy, bench_sqlite_fts5);
criterion_main!(benches);
