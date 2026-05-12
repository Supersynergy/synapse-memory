//! Criterion bench: synapse-space (via synapse-core Store) insert + FTS query
//! against the first 50 documents from lme_s_50.json.
//!
//! ChromaDB column: "could not install" — Python maturin link blocker (see RESULTS.md).

use std::path::PathBuf;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use serde::Deserialize;
use synapse_core::{db::Store, types::{PutRequest, SearchMode}};
use tempfile::NamedTempFile;

#[derive(Deserialize)]
struct LmeRecord {
    question_id: String,
    question: String,
    #[serde(default)]
    conversation_str: String,
}

fn load_records() -> Vec<LmeRecord> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../longmemeval/data/lme_s_50.json");
    let raw = std::fs::read_to_string(&p).expect("lme_s_50.json not found");
    serde_json::from_str(&raw).expect("parse lme_s_50.json")
}

fn bench_insert(c: &mut Criterion) {
    let records = load_records();
    c.bench_with_input(
        BenchmarkId::new("synapse_insert", records.len()),
        &records,
        |b, recs| {
            b.iter(|| {
                let f = NamedTempFile::new().unwrap();
                let mut store = Store::open(f.path()).unwrap();
                for r in recs {
                    let text = if r.conversation_str.is_empty() {
                        r.question.clone()
                    } else {
                        r.conversation_str[..r.conversation_str.len().min(2000)].to_string()
                    };
                    store.put(&PutRequest {
                        uri: Some(format!("lme://{}", r.question_id)),
                        title: Some(r.question[..r.question.len().min(120)].to_string()),
                        text,
                        meta: None,
                        embedding: None,
                    }).unwrap();
                }
            });
        },
    );
}

fn bench_query(c: &mut Criterion) {
    let records = load_records();
    // Pre-build the store once
    let f = NamedTempFile::new().unwrap();
    let mut store = Store::open(f.path()).unwrap();
    for r in &records {
        let text = if r.conversation_str.is_empty() {
            r.question.clone()
        } else {
            r.conversation_str[..r.conversation_str.len().min(2000)].to_string()
        };
        store.put(&PutRequest {
            uri: Some(format!("lme://{}", r.question_id)),
            title: Some(r.question[..r.question.len().min(120)].to_string()),
            text,
            meta: None,
            embedding: None,
        }).unwrap();
    }

    c.bench_with_input(
        BenchmarkId::new("synapse_query_lex", records.len()),
        &records,
        |b, recs| {
            b.iter(|| {
                for r in recs {
                    // FTS keyword query: first 5 alphanumeric words only (avoid FTS5 syntax errors)
                    let q: String = r.question
                        .split_whitespace()
                        .filter(|w| w.chars().all(|c| c.is_alphanumeric()))
                        .take(5)
                        .collect::<Vec<_>>()
                        .join(" ");
                    if q.is_empty() { continue; }
                    let _ = store.search(&q, SearchMode::Lex, None, 5).unwrap();
                }
            });
        },
    );
}

criterion_group!(benches, bench_insert, bench_query);
criterion_main!(benches);
