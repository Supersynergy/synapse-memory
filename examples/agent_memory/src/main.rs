//! Agent Memory — Mem0/Letta-killer demo
//!
//! Multi-scope memory (Global / User / Session) backed by Synapse hybrid
//! BM25+vec search with JSON metadata filter. No external services needed.
//!
//! # Run
//!   cargo run -- demo
//!   cargo run -- recall alice "what did I say about Rust"

use anyhow::Result;
use serde_json::json;
use std::time::Instant;
use synapse_core::{
    db::Store,
    types::{Hit, PutRequest, SearchMode},
};

const EMBED_DIM: usize = 384;

/// Deterministic pseudo-embedding from text (no model needed for demo).
/// Real deployment: swap for fastembed BGE-small → sub-ms.
fn pseudo_embed(text: &str) -> Vec<f32> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut out = vec![0.0f32; EMBED_DIM];
    for (i, chunk) in text.as_bytes().chunks(4).enumerate() {
        let mut h = DefaultHasher::new();
        chunk.hash(&mut h);
        (i as u64).hash(&mut h);
        let v = (h.finish() as f64 / u64::MAX as f64) * 2.0 - 1.0;
        out[i % EMBED_DIM] += v as f32;
    }
    let norm = out.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    out.iter_mut().for_each(|x| *x /= norm);
    out
}

/// Store one chat turn with scope metadata.
///
/// Meta keys:
///   user_id    → user-scoped recall
///   session_id → per-session context
///   scope      → "global" | "user" | "session"
fn record_interaction(
    store: &mut Store,
    user_id: &str,
    session_id: &str,
    role: &str,
    content: &str,
) -> Result<i64> {
    let text = format!("[{role}] {content}");
    let emb = pseudo_embed(&text);
    let req = PutRequest {
        title: Some(format!("{user_id}/{session_id}/{role}")),
        text,
        meta: Some(json!({
            "user_id":    user_id,
            "session_id": session_id,
            "scope":      "user",
            "role":       role,
        })),
        embedding: Some(emb),
        uri: None,
    };
    Ok(store.put(&req)?)
}

/// Hybrid recall restricted to a user's memories.
///
/// Uses BM25+vec RRF then filters by user_id prefix in title.
/// Production: use `SearchOptions` metadata filter pushdown.
fn recall(store: &Store, user_id: &str, query: &str, top_k: usize) -> Result<Vec<Hit>> {
    let q_emb = pseudo_embed(query);
    let hits = store.search(query, SearchMode::Hybrid, Some(&q_emb), top_k * 4)?;

    let filtered: Vec<Hit> = hits
        .into_iter()
        .filter(|h| {
            h.title
                .as_deref()
                .map(|t| t.starts_with(user_id))
                .unwrap_or(false)
        })
        .take(top_k)
        .collect();
    Ok(filtered)
}

fn run_demo() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let db_path = tmp.path().join("memory.db");
    let mut store = Store::open(&db_path)?;

    let user = "alice";
    let session = "s001";

    let turns: &[(&str, &str)] = &[
        ("user",      "I've been exploring Rust for systems programming. Love the borrow checker."),
        ("assistant", "Rust's ownership model eliminates whole classes of bugs at compile time."),
        ("user",      "Yeah. I also started learning async Rust with Tokio yesterday."),
        ("assistant", "Tokio is the go-to async runtime. tokio::spawn for task parallelism."),
        ("user",      "By the way, what's a good pizza place in Berlin?"),
    ];

    println!("── Recording {} turns  user={user}  session={session} ──", turns.len());
    let t0 = Instant::now();
    for (role, content) in turns {
        let id = record_interaction(&mut store, user, session, role, content)?;
        println!("  stored id={id}  [{role}] {}", &content[..content.len().min(60)]);
    }
    println!("Ingest: {:.1}ms\n", t0.elapsed().as_secs_f64() * 1000.0);

    let query = "remind me what I said about Rust";
    println!("── Recall  query=\"{query}\"  scope={user} ──");
    let t1 = Instant::now();
    let hits = recall(&store, user, query, 3)?;
    println!("Recall: {:.1}ms  {} hits\n", t1.elapsed().as_secs_f64() * 1000.0, hits.len());

    for (i, h) in hits.iter().enumerate() {
        println!("  #{} score={:.4}  {}", i + 1, h.score, h.text);
    }

    assert!(!hits.is_empty(), "expected at least 1 hit");
    let top = hits[0].text.to_lowercase();
    assert!(
        top.contains("rust") || top.contains("borrow") || top.contains("ownership") || top.contains("tokio"),
        "top hit should be Rust-related, got: {}",
        hits[0].text
    );
    println!("\n✓ demo passed — top hit is Rust-relevant");
    Ok(())
}

fn run_recall(user_id: &str, query: &str) -> Result<()> {
    let path = std::path::Path::new("memory.db");
    if !path.exists() {
        eprintln!("memory.db not found. Run `-- demo` first to populate a persistent store.");
        std::process::exit(1);
    }
    let store = Store::open(path)?;
    let hits = recall(&store, user_id, query, 5)?;
    println!("Recall [{user_id}] \"{query}\" → {} hits", hits.len());
    for (i, h) in hits.iter().enumerate() {
        println!("  #{} {:.4}  {}", i + 1, h.score, h.text);
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [cmd] if cmd == "demo" => run_demo(),
        [cmd, user, query] if cmd == "recall" => run_recall(user, query),
        _ => {
            eprintln!("Usage:");
            eprintln!("  cargo run -- demo");
            eprintln!("  cargo run -- recall <user_id> \"<query>\"");
            Ok(())
        }
    }
}
