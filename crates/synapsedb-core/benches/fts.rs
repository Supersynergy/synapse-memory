//! Criterion benchmark — Tantivy BM25 query latency.
//!
//! Run with:
//!   cargo bench -p synapse-core --features fts-tantivy --bench fts
//!
//! Hardware + rustc version are captured by `criterion` into
//! `target/criterion/**/report.json`. Publish under `docs/BENCH-<date>.md`.

use criterion::{black_box, criterion_group, criterion_main, Criterion};

#[cfg(feature = "fts-tantivy")]
fn bench_bm25(c: &mut Criterion) {
    use synapsedb_core::synx::fts::FtsIndex;

    const WORDS: &str = "rust ships ferris ownership borrow mcp memory vector embed synx \
tantivy hnsw blake3 zstd cow journal merkle scope session global supersedes references \
contradicts summarises agent claude crm event lead scraping research brainpack sqlite";

    let ws: Vec<&str> = WORDS.split_whitespace().collect();
    let phrase = |i: usize| -> String {
        (0..10)
            .map(|j| ws[(i + j) % ws.len()])
            .collect::<Vec<_>>()
            .join(" ")
    };

    let fts = FtsIndex::new().unwrap();
    let rows: Vec<(String, String, String, String)> = (0..10_000)
        .map(|i| {
            (
                format!("d{i}"),
                format!("doc {i}"),
                phrase(i),
                "global".into(),
            )
        })
        .collect();
    fts.write(&rows).unwrap();

    c.bench_function("bm25 unigram 10k docs", |b| {
        b.iter(|| {
            let _ = fts.search(black_box("rust"), 10).unwrap();
        })
    });
    c.bench_function("bm25 boolean OR 10k docs", |b| {
        b.iter(|| {
            let _ = fts
                .search(black_box("rust OR tantivy OR vector"), 10)
                .unwrap();
        })
    });
    c.bench_function("bm25 phrase 10k docs", |b| {
        b.iter(|| {
            let _ = fts
                .search(black_box("\"rust ships\""), 10)
                .unwrap_or_default();
        })
    });
}

#[cfg(not(feature = "fts-tantivy"))]
fn bench_bm25(_c: &mut Criterion) {
    eprintln!("skip: enable --features fts-tantivy");
}

criterion_group!(benches, bench_bm25);
criterion_main!(benches);
