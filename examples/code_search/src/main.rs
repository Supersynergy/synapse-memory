//! Code Search — Cursor/codebase-RAG-killer demo
//!
//! Index .rs files → hybrid BM25 (FTS5) + ColBERT i8-quantised multi-vector.
//! Returns top-5 file:line hits with millisecond latency.
//!
//! # Run
//!   cargo run -- index .
//!   cargo run -- search "function that parses tokens"

use anyhow::{Context, Result};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use synapse_colbert::{ColbertEmbedder, ColbertStore};
use synapse_core::{
    db::Store,
    types::{Hit, PutRequest, SearchMode},
};

const DB_PATH: &str = "code_search.db";

// ── helpers ──────────────────────────────────────────────────────────────────

fn pseudo_embed(text: &str) -> Vec<f32> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let dim = 384usize;
    let mut out = vec![0.0f32; dim];
    for (i, chunk) in text.as_bytes().chunks(4).enumerate() {
        let mut h = DefaultHasher::new();
        chunk.hash(&mut h);
        (i as u64).hash(&mut h);
        let v = (h.finish() as f64 / u64::MAX as f64) * 2.0 - 1.0;
        out[i % dim] += v as f32;
    }
    let norm = out.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    out.iter_mut().for_each(|x| *x /= norm);
    out
}

fn collect_rs(root: &Path) -> Vec<PathBuf> {
    fn walk(p: &Path, acc: &mut Vec<PathBuf>) {
        if let Ok(rd) = fs::read_dir(p) {
            for e in rd.flatten() {
                let path = e.path();
                if path.is_dir() {
                    let skip = matches!(
                        path.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                        "target" | ".git"
                    );
                    if !skip { walk(&path, acc); }
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    acc.push(path);
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out
}

/// 30-line windows, 20-line step (overlap for recall).
fn chunk_file(content: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = content.lines().collect();
    let (window, step) = (30, 20);
    let mut chunks = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        let end = (i + window).min(lines.len());
        let text = lines[i..end].join("\n");
        if !text.trim().is_empty() {
            chunks.push((i + 1, text));
        }
        if end == lines.len() { break; }
        i += step;
    }
    chunks
}

// ── commands ──────────────────────────────────────────────────────────────────

fn cmd_index(root: &str) -> Result<()> {
    let root = Path::new(root);
    let files = collect_rs(root);
    println!("Found {} .rs files under {}", files.len(), root.display());

    let mut store = Store::open(DB_PATH)?;
    let colbert_emb = ColbertEmbedder::default();

    let t0 = Instant::now();

    // Pass 1: insert all chunks via Store::put, collect (doc_id, chunk_text).
    let mut id_chunks: Vec<(i64, String)> = Vec::new();
    for path in &files {
        let content = match fs::read_to_string(path) { Ok(c) => c, Err(_) => continue };
        let rel = path.strip_prefix(root).unwrap_or(path);
        for (line_no, chunk) in chunk_file(&content) {
            let uri = format!("{}:{}", rel.display(), line_no);
            let doc_id = store.put(&PutRequest {
                uri: Some(uri.clone()),
                title: Some(uri),
                text: chunk.clone(),
                meta: Some(json!({ "file": rel.to_string_lossy(), "line": line_no, "lang": "rust" })),
                embedding: Some(pseudo_embed(&chunk)),
            })?;
            id_chunks.push((doc_id, chunk));
        }
    }

    // Pass 2: ColBERT multi-vector (borrows store.conn only here, no conflict).
    let colbert_store = ColbertStore::new(&store.conn)?;
    for (doc_id, chunk) in &id_chunks {
        let token_vecs = colbert_emb.embed_doc(chunk)?;
        colbert_store.add_colbert_i8(*doc_id, token_vecs)?;
    }

    let secs = t0.elapsed().as_secs_f64();
    println!(
        "Indexed {} chunks from {} files in {:.1}ms ({:.0} chunks/s)\nDB: {DB_PATH}",
        id_chunks.len(), files.len(), secs * 1000.0, id_chunks.len() as f64 / secs
    );
    Ok(())
}

fn cmd_search(query: &str) -> Result<()> {
    let store = Store::open(DB_PATH).context("Run `index <dir>` first")?;
    let colbert_emb = ColbertEmbedder::default();
    let colbert_store = ColbertStore::new(&store.conn)?;

    let t0 = Instant::now();
    let hits = store.search(query, SearchMode::Hybrid, Some(&pseudo_embed(query)), 20)?;
    let hybrid_ms = t0.elapsed().as_secs_f64() * 1000.0;

    let candidate_ids: Vec<i64> = hits.iter().map(|h| h.id).collect();
    let t1 = Instant::now();
    let mut reranked = colbert_store.colbert_rerank_i8(query, &candidate_ids)?;
    reranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    let rerank_ms = t1.elapsed().as_secs_f64() * 1000.0;

    let hit_map: HashMap<i64, &Hit> = hits.iter().map(|h| (h.id, h)).collect();

    println!("Query: \"{query}\"  hybrid={hybrid_ms:.1}ms  colbert-rerank={rerank_ms:.1}ms\n");

    let top5: Vec<_> = reranked.iter().take(5).collect();
    if top5.is_empty() {
        println!("No results. Run `index <dir>` first.");
        return Ok(());
    }
    for (rank, (id, score)) in top5.iter().enumerate() {
        if let Some(h) = hit_map.get(id) {
            let loc = h.uri.as_deref().unwrap_or("?");
            let preview: String = h.text.lines().take(2).collect::<Vec<_>>().join(" | ");
            println!(
                "  #{} [{:.4}] {loc}\n      {}",
                rank + 1, score,
                &preview[..preview.len().min(120)]
            );
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [cmd, path] if cmd == "index" => cmd_index(path),
        [cmd, query] if cmd == "search" => cmd_search(query),
        _ => {
            eprintln!("Usage:");
            eprintln!("  cargo run -- index <dir>");
            eprintln!("  cargo run -- search \"<query>\"");
            Ok(())
        }
    }
}
