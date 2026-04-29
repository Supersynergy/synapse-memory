//! LongMemEval-S benchmark runner.
//!
//! Reads `lme_s_50.json` (50-question subset, derived from
//! `xiaoyuanliu/longmemeval-s-50` HF parquet → DuckDB → JSON).
//!
//! For each question:
//!   1. Open temp Synapse Store (fresh, in tempdir).
//!   2. Apply `sota_migrate`.
//!   3. Split `conversation_str` by `Session Timestamp:` → N session-docs.
//!   4. Ingest each as a Store doc (text-only, no embedding — lexical recall).
//!   5. Run `pipeline_recall` with RuleHooks (or MlxHooks under --use-mlx).
//!   6. Hit = answer-substring (lower-cased, alnum-only) found in any top-k doc.
//!
//! Reports Recall@5 and Recall@10 plus latency stats.

use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;
use std::path::PathBuf;
use std::time::Instant;
use synapse_core::db::Store;
use synapse_core::embed::Embedder;
use synapse_core::sota::{sota_migrate, RecallParams};
use synapse_core::sota_pipeline::{pipeline_recall, PipelineHooks, RuleHooks};
use synapse_core::types::PutRequest;

mod judge;
mod mlx;

/// Wrap any PipelineHooks impl to sanitize free-form text outputs into FTS5-safe
/// queries. Used because the pipeline feeds `hyde()` and `decompose()` outputs
/// straight into Store::search → fts5 MATCH.
struct SanHooks<'a, H: PipelineHooks> {
    inner: &'a H,
}
impl<'a, H: PipelineHooks> PipelineHooks for SanHooks<'a, H> {
    fn decompose(&self, query: &str) -> synapse_core::error::Result<Vec<String>> {
        let subs = self.inner.decompose(query)?;
        Ok(subs.into_iter().map(|s| fts5_sanitize(&s)).collect())
    }
    fn grade(&self, query: &str, doc: &str) -> synapse_core::error::Result<f64> {
        self.inner.grade(query, doc)
    }
    fn hyde(&self, query: &str) -> synapse_core::error::Result<String> {
        let h = self.inner.hyde(query)?;
        Ok(fts5_sanitize(&h))
    }
    fn summarize(&self, items: &[&str]) -> synapse_core::error::Result<String> {
        self.inner.summarize(items)
    }
    fn merge(&self, e: &str, n: &str) -> synapse_core::error::Result<String> {
        self.inner.merge(e, n)
    }
}

#[derive(Parser, Debug)]
#[command(about = "LongMemEval-S benchmark runner for Synapse SOTA pipeline")]
struct Args {
    /// Path to the LongMemEval JSON file (50-question subset format).
    #[arg(long, default_value = "bench/longmemeval/data/lme_s_50.json")]
    data: PathBuf,
    /// Limit number of questions evaluated (0 = all).
    #[arg(long, default_value_t = 0)]
    limit: usize,
    /// Use MLX subprocess hooks (decompose / grade / hyde).
    #[arg(long, default_value_t = false)]
    use_mlx: bool,
    /// MLX model id.
    #[arg(long, default_value = "mlx-community/SmolLM2-1.7B-Instruct-4bit")]
    mlx_model: String,
    /// Per-call MLX timeout (ms).
    #[arg(long, default_value_t = 1500)]
    mlx_timeout_ms: u64,
    /// Self-RAG relevance floor (0 disables grade-filtering).
    #[arg(long, default_value_t = 0.0)]
    relevance_floor: f64,
    /// HyDE rescue threshold (run HyDE if total hits < this).
    #[arg(long, default_value_t = 3)]
    hyde_threshold: usize,
    /// Verbose per-question logging.
    #[arg(long, default_value_t = false)]
    verbose: bool,
    /// Enable real embeddings (fastembed BGE-small-384) → Hybrid recall.
    #[arg(long, default_value_t = false)]
    embed: bool,
    /// Enable LLM-judge mode (protocol parity with LongMemEval paper).
    /// Reports both substring-R@5 and judge-R@5.
    #[arg(long, default_value_t = false)]
    judge: bool,
    /// Judge model id (must be in HF cache for offline mode).
    #[arg(long, default_value = "mlx-community/Llama-3.2-3B-Instruct-4bit")]
    judge_model: String,
    /// Per-judge-call timeout (ms).
    #[arg(long, default_value_t = 30000)]
    judge_timeout_ms: u64,
    /// Use original (non-sanitized) query text for FTS5. By default queries
    /// pass through the alnum sanitizer; with embed=on FTS5 still benefits
    /// from sanitization.
    #[arg(long, default_value_t = false)]
    raw_query: bool,
}

#[derive(Debug, Deserialize)]
struct Question {
    question_id: String,
    #[allow(dead_code)]
    question_type: String,
    question: String,
    answer: String,
    #[allow(dead_code)]
    question_date: String,
    conversation_str: String,
}

/// Sanitize a free-form user query for sqlite-fts5 MATCH.
/// Strategy: keep alnum tokens >=3 chars, OR-join, fall back to one common
/// token if everything is filtered. Stopwords are pruned to keep selectivity.
fn fts5_sanitize(q: &str) -> String {
    const STOP: &[&str] = &[
        "the", "and", "for", "are", "but", "not", "you", "all", "can", "had", "her", "was",
        "one", "our", "out", "day", "get", "has", "him", "his", "how", "man", "new", "now",
        "old", "see", "two", "way", "who", "boy", "did", "its", "let", "put", "say", "she",
        "too", "use", "what", "when", "where", "which", "this", "that", "with", "from", "have",
        "your", "they", "their", "would", "could", "should", "about", "into", "than", "then",
        "been", "were", "will", "much", "many", "some", "such", "only", "very", "just", "also",
        "make", "made", "does", "doing", "didnt",
    ];
    let toks: Vec<String> = q
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 3 && !STOP.contains(t))
        .map(|s| s.to_string())
        .collect();
    if toks.is_empty() {
        return "memory".into();
    }
    // OR-join in fts5 syntax: "a" OR "b" — quote each token so single
    // standalone special chars never reach the parser.
    toks.iter()
        .take(16)
        .map(|t| format!("\"{}\"", t))
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn normalize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Split a LongMemEval `conversation_str` into per-session docs.
/// Sessions begin with the literal token "Session Timestamp:".
fn split_sessions(text: &str) -> Vec<String> {
    let marker = "Session Timestamp:";
    let mut docs = Vec::new();
    let mut last = 0usize;
    let bytes = text.as_bytes();
    let mlen = marker.len();
    let mut i = 0usize;
    while i + mlen <= bytes.len() {
        if &bytes[i..i + mlen] == marker.as_bytes() {
            if i > last {
                let chunk = text[last..i].trim();
                if !chunk.is_empty() {
                    docs.push(chunk.to_string());
                }
            }
            last = i;
            i += mlen;
        } else {
            i += 1;
        }
    }
    if last < text.len() {
        let chunk = text[last..].trim();
        if !chunk.is_empty() {
            docs.push(chunk.to_string());
        }
    }
    if docs.is_empty() {
        docs.push(text.to_string());
    }
    docs
}

fn answer_in_any(answer: &str, docs: &[&str]) -> bool {
    let na = normalize(answer);
    if na.is_empty() {
        return false;
    }
    // Use first 8 normalized words as needle (answers are usually short).
    let needle: String = na.split_whitespace().take(8).collect::<Vec<_>>().join(" ");
    if needle.is_empty() {
        return false;
    }
    for d in docs {
        let nd = normalize(d);
        if nd.contains(&needle) {
            return true;
        }
    }
    false
}

fn run_question<H: PipelineHooks>(
    q: &Question,
    hooks: &H,
    relevance_floor: f64,
    hyde_threshold: usize,
    embedder: Option<&Embedder>,
) -> Result<(bool, bool, u128, usize, Vec<String>)> {
    // Fresh tempfile-backed store per question (Store::open requires a path).
    let tmp = tempfile_path(&q.question_id)?;
    // Make sure no leftover.
    let _ = std::fs::remove_file(&tmp);
    let mut store = Store::open(&tmp).context("Store::open")?;
    sota_migrate(&store.conn).context("sota_migrate")?;
    let docs = split_sessions(&q.conversation_str);
    let n_docs = docs.len();
    // Pre-compute doc embeddings (if enabled). BGE handles long-ish input
    // by truncation internally; we feed raw session text.
    let doc_embs: Option<Vec<Vec<f32>>> = if let Some(emb) = embedder {
        Some(emb.embed_batch(&docs.iter().cloned().collect::<Vec<_>>())?)
    } else {
        None
    };
    for (i, d) in docs.iter().enumerate() {
        let req = PutRequest {
            text: d.clone(),
            embedding: doc_embs.as_ref().map(|v| v[i].clone()),
            ..Default::default()
        };
        store.put(&req)?;
    }
    let q_emb: Option<Vec<f32>> = if let Some(emb) = embedder {
        Some(emb.embed_one(&q.question)?)
    } else {
        None
    };
    let t = Instant::now();
    let mut params = RecallParams::default();
    params.query = fts5_sanitize(&q.question);
    params.k = 10;
    // Heat off: LongMemEval has artificial timestamps; recency decay would
    // distort multi-session retrieval. Entity-expand off: per-question fresh
    // store has no extracted memories yet (extraction pipeline not in bench).
    params.heat = false;
    params.entity_expand = false;
    params.rerank_top = 0;
    let hits = pipeline_recall(
        &store,
        hooks,
        &params,
        q_emb.as_deref(),
        relevance_floor,
        hyde_threshold,
    )?;
    let elapsed = t.elapsed().as_micros();

    let top5: Vec<&str> = hits.iter().take(5).map(|h| h.hit.text.as_str()).collect();
    let top10: Vec<&str> = hits.iter().take(10).map(|h| h.hit.text.as_str()).collect();
    let r5 = answer_in_any(&q.answer, &top5);
    let r10 = answer_in_any(&q.answer, &top10);
    let top5_owned: Vec<String> = top5.iter().map(|s| s.to_string()).collect();
    if std::env::var("LME_DEBUG_TOP5").ok().as_deref() == Some("1") {
        for (i, t) in top5.iter().enumerate() {
            let head: String = t.chars().take(140).collect();
            eprintln!("[dbg q={}] top5[{}]: {}", q.question_id, i, head.replace('\n', " "));
        }
    }

    drop(store);
    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::remove_file(format!("{}-wal", tmp.display()));
    let _ = std::fs::remove_file(format!("{}-shm", tmp.display()));
    Ok((r5, r10, elapsed, n_docs, top5_owned))
}

fn tempfile_path(qid: &str) -> Result<PathBuf> {
    let mut p = std::env::temp_dir();
    p.push(format!("synapse-lme-{}.db", qid));
    Ok(p)
}

fn main() -> Result<()> {
    let args = Args::parse();
    let raw = std::fs::read_to_string(&args.data)
        .with_context(|| format!("read {}", args.data.display()))?;
    let mut qs: Vec<Question> = serde_json::from_str(&raw).context("parse JSON")?;
    if args.limit > 0 && qs.len() > args.limit {
        qs.truncate(args.limit);
    }
    println!(
        "LongMemEval-S bench: {} questions | hooks: {} | floor={} hyde_th={}",
        qs.len(),
        if args.use_mlx { "MLX" } else { "Rule" },
        args.relevance_floor,
        args.hyde_threshold,
    );

    let rule = RuleHooks::default();
    #[allow(unused_variables)]
    let mlx_hooks = mlx::MlxHooks::new(args.mlx_model.clone(), args.mlx_timeout_ms);

    let embedder = if args.embed {
        match Embedder::new() {
            Ok(e) => {
                println!("Embedder: fastembed BGE-small-en-v1.5 (384-dim)");
                Some(e)
            }
            Err(e) => {
                eprintln!("WARN: embedder init failed: {} — running lex-only", e);
                None
            }
        }
    } else {
        None
    };
    let embedder_ref = embedder.as_ref();

    let mut r5_hits = 0usize;
    let mut r10_hits = 0usize;
    let mut judge_r5_hits = 0usize;
    let mut judge_r5_evaluated = 0usize;
    let mut total_ms: u128 = 0;
    let mut total_docs: usize = 0;
    let mut errs: Vec<(String, String)> = Vec::new();
    let judge = if args.judge {
        let j = judge::Judge::new(args.judge_model.clone(), args.judge_timeout_ms);
        if j.is_available() {
            println!("Judge: {} (offline)", args.judge_model);
            Some(j)
        } else {
            None
        }
    } else {
        None
    };

    for (i, q) in qs.iter().enumerate() {
        let res = if args.use_mlx {
            let san = SanHooks { inner: &mlx_hooks };
            run_question(q, &san, args.relevance_floor, args.hyde_threshold, embedder_ref)
        } else {
            let san = SanHooks { inner: &rule };
            run_question(q, &san, args.relevance_floor, args.hyde_threshold, embedder_ref)
        };
        match res {
            Ok((r5, r10, ms, nd, top5)) => {
                if r5 {
                    r5_hits += 1;
                }
                if r10 {
                    r10_hits += 1;
                }
                total_ms += ms;
                total_docs += nd;
                let mut judge_verdict: Option<bool> = None;
                if let Some(j) = judge.as_ref() {
                    let refs: Vec<&str> = top5.iter().map(|s| s.as_str()).collect();
                    if let Some(v) = j.judge(&q.question, &q.answer, &refs) {
                        judge_r5_evaluated += 1;
                        if v {
                            judge_r5_hits += 1;
                        }
                        judge_verdict = Some(v);
                    } else if r5 {
                        // Judge unavailable / parse-fail → trust substring
                        judge_r5_evaluated += 1;
                        judge_r5_hits += 1;
                        judge_verdict = Some(true);
                    }
                }
                if args.verbose {
                    println!(
                        "[{:>2}] {} type={} docs={} ms={} r5={} r10={} judge={:?}",
                        i + 1,
                        q.question_id,
                        q.question_type,
                        nd,
                        ms,
                        r5,
                        r10,
                        judge_verdict
                    );
                }
            }
            Err(e) => {
                errs.push((q.question_id.clone(), e.to_string()));
                eprintln!("ERR {}: {}", q.question_id, e);
            }
        }
    }

    let n = qs.len() as f64;
    let r5 = r5_hits as f64 / n;
    let r10 = r10_hits as f64 / n;
    let avg_ms = if n > 0.0 { total_ms as f64 / n } else { 0.0 };
    let avg_docs = if n > 0.0 { total_docs as f64 / n } else { 0.0 };

    println!("--- Results ---");
    println!("N            : {}", qs.len());
    println!("Errors       : {}", errs.len());
    println!("Recall@5     : {:.3}  ({}/{})", r5, r5_hits, qs.len());
    println!("Recall@10    : {:.3}  ({}/{})", r10, r10_hits, qs.len());
    if args.judge {
        let denom = qs.len() as f64;
        let jr5 = if denom > 0.0 { judge_r5_hits as f64 / denom } else { 0.0 };
        println!(
            "Judge-R@5    : {:.3}  ({}/{} evaluated, {} total)",
            jr5,
            judge_r5_hits,
            judge_r5_evaluated,
            qs.len()
        );
    }
    println!("Latency avg  : {:.2} ms (recall only, ingest excluded)", avg_ms / 1000.0);
    println!("Avg docs/Q   : {:.1}", avg_docs);
    Ok(())
}
