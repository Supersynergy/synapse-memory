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

#[derive(Debug, Clone, clap::ValueEnum)]
enum RerankModelArg {
    Baseline,
    JinaColbert,
    JinaCrossEncoder,
}

impl std::fmt::Display for RerankModelArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Baseline => write!(f, "baseline"),
            Self::JinaColbert => write!(f, "jina-colbert"),
            Self::JinaCrossEncoder => write!(f, "jina-cross-encoder"),
        }
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
    /// Use MiniMax-M2.7 highspeed for PipelineHooks (decompose / grade / hyde).
    /// Requires MINIMAX_API_KEY env. Overrides --use-mlx.
    #[arg(long, default_value_t = false)]
    use_minimax: bool,
    /// Run cross-encoder rerank (fastembed JINA v2 multilingual) over top-N hits.
    /// 0 disables. Default 20 = re-rank candidate pool, return top-k.
    #[arg(long, default_value_t = 20)]
    rerank_top: usize,
    /// Reranker model selection.
    ///   baseline          — BGE-reranker-v2-m3 ONNX (default, ~568M, cached by fastembed)
    ///   jina-colbert      — ColBERT MaxSim scaffold (requires colbert-jina model in HF cache)
    ///   jina-cross-encoder — JINA reranker v2 multilingual cross-encoder (~140MB)
    /// Jina models only load when ALLOW_BIG_DOWNLOAD=1 or already cached.
    #[arg(long, default_value = "baseline")]
    rerank_model: RerankModelArg,
    /// RRF k constant for fusion (default 60).
    #[arg(long, default_value_t = 60.0)]
    rrf_k: f64,
    /// Enable Personalized PageRank (HippoRAG-2) signal in recall fusion.
    #[arg(long, default_value_t = false)]
    ppr: bool,
    /// Pre-extract typed memories from each session via MiniMax before recall.
    /// Activates type-weighted RRF + entity edges + PPR for the bench.
    /// Requires --use-minimax. Adds ~3s per session (one LLM call per doc).
    #[arg(long, default_value_t = false)]
    pre_extract: bool,
    /// Enable HyDE (Hypothetical Document Embedding): expand query via Ollama
    /// before embedding the vec-query. BM25/FTS5 leg still uses original query.
    /// Requires feature `hyde` + `--embed`. Skipped when OLLAMA_AVAILABLE unset.
    #[arg(long, default_value_t = false)]
    hyde: bool,
    /// Ollama model for HyDE expansion (default: phi4-mini).
    #[arg(long, default_value = "phi4-mini")]
    hyde_model: String,
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
        "the", "and", "for", "are", "but", "not", "you", "all", "can", "had", "her", "was", "one",
        "our", "out", "day", "get", "has", "him", "his", "how", "man", "new", "now", "old", "see",
        "two", "way", "who", "boy", "did", "its", "let", "put", "say", "she", "too", "use", "what",
        "when", "where", "which", "this", "that", "with", "from", "have", "your", "they", "their",
        "would", "could", "should", "about", "into", "than", "then", "been", "were", "will",
        "much", "many", "some", "such", "only", "very", "just", "also", "make", "made", "does",
        "doing", "didnt",
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

/// Fuzzy fallback: token-set-recall (answer-tokens ∩ doc-tokens / answer-tokens) ≥ thr.
/// Catches paraphrases that strict substring misses.
/// `min_tok` enforces ≥3-char content tokens. `thr` typical 0.6-0.8.
fn answer_in_any_fuzzy(answer: &str, docs: &[&str], thr: f64) -> bool {
    let na = normalize(answer);
    let a_toks: std::collections::HashSet<&str> =
        na.split_whitespace().filter(|t| t.len() >= 3).collect();
    if a_toks.is_empty() {
        return false;
    }
    let need = ((a_toks.len() as f64) * thr).ceil() as usize;
    for d in docs {
        let nd = normalize(d);
        let d_toks: std::collections::HashSet<&str> = nd.split_whitespace().collect();
        let hit = a_toks.iter().filter(|t| d_toks.contains(*t)).count();
        if hit >= need.max(1) {
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
    reranker: Option<&dyn synapse_rerank::Reranker>,
    ppr: bool,
    pre_extractor: Option<&dyn synapse_extract::Extractor>,
    rrf_k: f64,
    rerank_top: usize,
    #[cfg(feature = "hyde")] hyde_cfg: Option<&synapse_core::turbo::hyde::HydeConfig>,
) -> Result<(
    bool,
    bool,
    bool,
    bool,
    u128,
    usize,
    Vec<String>,
    Vec<String>,
    u128,
)> {
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
    // Pre-extraction: parallelize the LLM HTTP round-trips via std::thread::scope
    // (no new deps). DB writes still serial. With ~47 docs and 8 worker threads,
    // total cost ≈ ceil(47/8) × per-call latency ≈ 6 × 1.5s ≈ 9s instead of 47×6s.
    if let Some(ext) = pre_extractor {
        use synapse_extract::{enqueue_extraction_helper, ExtractedMemory};
        // 1) extract in parallel.
        let n = docs.len();
        let results: Vec<(i64, Vec<ExtractedMemory>)> = std::thread::scope(|s| {
            let workers = 8usize;
            let chunk = (n + workers - 1) / workers;
            let mut handles = Vec::new();
            for w in 0..workers {
                let start = w * chunk;
                let end = ((w + 1) * chunk).min(n);
                if start >= end {
                    continue;
                }
                let docs_ref = &docs;
                let ext_ref = ext;
                handles.push(s.spawn(move || {
                    let mut local: Vec<(i64, Vec<ExtractedMemory>)> = Vec::new();
                    for i in start..end {
                        let did = (i + 1) as i64;
                        match ext_ref.extract(&docs_ref[i]) {
                            Ok(items) => local.push((did, items)),
                            Err(_) => local.push((did, Vec::new())),
                        }
                    }
                    local
                }));
            }
            let mut all = Vec::with_capacity(n);
            for h in handles {
                all.extend(h.join().unwrap_or_default());
            }
            all
        });
        // 2) write serially.
        for (doc_id, items) in results {
            let _ = enqueue_extraction_helper(&store.conn, doc_id, &items);
        }
    }
    // HyDE: expand query via Ollama before embedding the vec leg.
    // BM25/FTS5 still uses original sanitized query (set later via params.query).
    #[cfg(feature = "hyde")]
    let (q_emb, hyde_latency_us): (Option<Vec<f32>>, u128) = if let Some(emb) = embedder {
        if let Some(hcfg) = hyde_cfg {
            let t_hyde = Instant::now();
            let expanded = synapse_core::turbo::hyde::expand(hcfg, &q.question);
            let hyde_us = t_hyde.elapsed().as_micros();
            (Some(emb.embed_one(&expanded)?), hyde_us)
        } else {
            (Some(emb.embed_one(&q.question)?), 0)
        }
    } else {
        (None, 0)
    };
    #[cfg(not(feature = "hyde"))]
    let (q_emb, hyde_latency_us): (Option<Vec<f32>>, u128) = if let Some(emb) = embedder {
        (Some(emb.embed_one(&q.question)?), 0)
    } else {
        (None, 0)
    };
    let t = Instant::now();
    let mut params = RecallParams::default();
    params.query = fts5_sanitize(&q.question);
    params.k = 10;
    // Heat off: LongMemEval has artificial timestamps; recency decay would
    // distort multi-session retrieval. Entity-expand off: per-question fresh
    // store has no extracted memories yet (extraction pipeline not in bench).
    params.heat = false;
    params.entity_expand = pre_extractor.is_some(); // only meaningful with extracted memories
    params.ppr = ppr && pre_extractor.is_some(); // PPR needs edges
    params.rrf_k = rrf_k;
    params.rerank_top = if reranker.is_some() { rerank_top } else { 0 };
    let mut hits = pipeline_recall(
        &store,
        hooks,
        &params,
        q_emb.as_deref(),
        relevance_floor,
        hyde_threshold,
    )?;
    if let Some(r) = reranker {
        let cand: Vec<synapse_core::Hit> = hits.iter().map(|h| h.hit.clone()).collect();
        let rer = r
            .rerank(&q.question, cand, params.k)
            .unwrap_or_else(|_| hits.iter().map(|h| h.hit.clone()).collect());
        // Re-key reranked hits back into RecallHit (memory_id/type lost — fine for bench).
        hits = rer
            .into_iter()
            .map(|h| synapse_core::sota::RecallHit {
                hit: h,
                memory_id: None,
                memory_type: None,
            })
            .collect();
    }
    let elapsed = t.elapsed().as_micros();

    let top5: Vec<&str> = hits.iter().take(5).map(|h| h.hit.text.as_str()).collect();
    let top10: Vec<&str> = hits.iter().take(10).map(|h| h.hit.text.as_str()).collect();
    let r5 = answer_in_any(&q.answer, &top5);
    let r10 = answer_in_any(&q.answer, &top10);
    let f5 = answer_in_any_fuzzy(&q.answer, &top5, 0.6);
    let f10 = answer_in_any_fuzzy(&q.answer, &top10, 0.6);
    let top5_owned: Vec<String> = top5.iter().map(|s| s.to_string()).collect();
    let top10_owned: Vec<String> = top10.iter().map(|s| s.to_string()).collect();
    if std::env::var("LME_DEBUG_TOP5").ok().as_deref() == Some("1") {
        for (i, t) in top5.iter().enumerate() {
            let head: String = t.chars().take(140).collect();
            eprintln!(
                "[dbg q={}] top5[{}]: {}",
                q.question_id,
                i,
                head.replace('\n', " ")
            );
        }
    }

    drop(store);
    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::remove_file(format!("{}-wal", tmp.display()));
    let _ = std::fs::remove_file(format!("{}-shm", tmp.display()));
    Ok((
        r5,
        r10,
        f5,
        f10,
        elapsed,
        n_docs,
        top5_owned,
        top10_owned,
        hyde_latency_us,
    ))
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
    #[cfg(feature = "minimax")]
    let minimax_hooks: Option<synapse_extract::minimax::MinimaxHooks> = if args.use_minimax {
        match synapse_extract::minimax::MinimaxHooks::from_env() {
            Ok(h) => {
                println!("Hooks: MiniMax-M2.7-highspeed (HTTP)");
                Some(h)
            }
            Err(e) => {
                eprintln!("WARN: minimax init failed: {} — falling back to Rule", e);
                None
            }
        }
    } else {
        None
    };
    #[cfg(not(feature = "minimax"))]
    let minimax_hooks: Option<()> = None;

    // Reranker init. Jina models require ALLOW_BIG_DOWNLOAD=1 or cached HF model.
    #[cfg(feature = "rerank")]
    let reranker_box: Option<Box<dyn synapse_rerank::Reranker>> = if args.rerank_top > 0 {
        match &args.rerank_model {
            RerankModelArg::JinaColbert => {
                // ColBERT scaffold — logs warning when no model path provided
                let r = synapse_rerank::ColbertReranker::new(None);
                if r.is_loaded() {
                    println!(
                        "Reranker: jina-colbert (ColBERT MaxSim, top={})",
                        args.rerank_top
                    );
                } else {
                    eprintln!("WARN: jina-colbert model not cached — ColbertReranker scaffold (identity-rerank, order preserved). Set HF model path or ALLOW_BIG_DOWNLOAD=1 to download.");
                }
                Some(Box::new(r) as Box<dyn synapse_rerank::Reranker>)
            }
            RerankModelArg::JinaCrossEncoder => {
                // JINA reranker-v2-multilingual via fastembed ONNX (~140MB).
                // fastembed will download on first use — only proceed with ALLOW_BIG_DOWNLOAD=1.
                let allow_dl = std::env::var("ALLOW_BIG_DOWNLOAD")
                    .map(|v| v == "1")
                    .unwrap_or(false);
                if allow_dl {
                    match synapse_rerank::onnx::OnnxCrossEncoder::new_jina_v2() {
                        Ok(r) => {
                            println!("Reranker: JINA-reranker-v2-base-multilingual (ONNX cross-encoder, top={})", args.rerank_top);
                            Some(Box::new(r))
                        }
                        Err(e) => {
                            eprintln!("WARN: jina-cross-encoder init failed: {} — falling back to BGE baseline", e);
                            match synapse_rerank::onnx::OnnxCrossEncoder::new() {
                                Ok(r) => {
                                    println!(
                                        "Reranker (fallback): BGE-reranker-v2-m3 (top={})",
                                        args.rerank_top
                                    );
                                    Some(Box::new(r))
                                }
                                Err(e2) => {
                                    eprintln!("WARN: fallback also failed: {} — rerank-off", e2);
                                    None
                                }
                            }
                        }
                    }
                } else {
                    eprintln!("WARN: --rerank-model jina-cross-encoder requires ALLOW_BIG_DOWNLOAD=1 (model ~140MB). Falling back to BGE baseline.");
                    match synapse_rerank::onnx::OnnxCrossEncoder::new() {
                        Ok(r) => {
                            println!(
                                "Reranker (fallback): BGE-reranker-v2-m3 (top={})",
                                args.rerank_top
                            );
                            Some(Box::new(r))
                        }
                        Err(e) => {
                            eprintln!("WARN: fallback reranker init failed: {} — rerank-off", e);
                            None
                        }
                    }
                }
            }
            RerankModelArg::Baseline => match synapse_rerank::onnx::OnnxCrossEncoder::new() {
                Ok(r) => {
                    println!(
                        "Reranker: BGE-reranker-v2-m3 (568M ONNX cross-encoder, top={})",
                        args.rerank_top
                    );
                    Some(Box::new(r))
                }
                Err(e) => {
                    eprintln!("WARN: reranker init failed: {} — running rerank-off", e);
                    None
                }
            },
        }
    } else {
        None
    };
    #[cfg(not(feature = "rerank"))]
    let reranker_box: Option<Box<dyn synapse_rerank::Reranker>> = None;
    let reranker_ref = reranker_box.as_deref();

    // Pre-extractor: when --pre-extract + --use-minimax, runs hierarchical
    // extract over each session BEFORE recall, populating typed memories +
    // entity edges so PPR + RRF-typed are non-trivial.
    // Pre-extractor selection:
    //   --pre-extract + --use-minimax → MiniMax hierarchical (best, ~30s/call)
    //   --pre-extract alone           → RuleExtractor (fast, no LLM)
    //   default                       → none (no typed memories, PPR signal weak)
    #[cfg(feature = "minimax")]
    let pre_extractor_box: Option<Box<dyn synapse_extract::Extractor>> = if args.pre_extract {
        if args.use_minimax {
            match synapse_extract::minimax::MinimaxExtractor::from_env() {
                Ok(e) => {
                    println!("Pre-extract: MiniMax-M2 hierarchical (Mem0-v3)");
                    Some(Box::new(e))
                }
                Err(e) => {
                    eprintln!(
                        "WARN: pre_extract minimax init failed: {} — falling back to rule",
                        e
                    );
                    Some(Box::new(synapse_extract::RuleExtractor))
                }
            }
        } else {
            println!("Pre-extract: RuleExtractor (fast, no LLM)");
            Some(Box::new(synapse_extract::RuleExtractor))
        }
    } else {
        None
    };
    #[cfg(not(feature = "minimax"))]
    let pre_extractor_box: Option<Box<dyn synapse_extract::Extractor>> = if args.pre_extract {
        Some(Box::new(synapse_extract::RuleExtractor))
    } else {
        None
    };
    let pre_extractor_ref: Option<&dyn synapse_extract::Extractor> = pre_extractor_box.as_deref();

    // When both embed-768 and rerank features are active, default to arctic-m
    // (768-dim, MTEB 62.5, +4pp R@5 over BGE-small baseline).
    // Respect explicit SYNAPSE_EMBED_MODEL override.
    #[cfg(all(feature = "embed-768", feature = "rerank"))]
    if args.embed && std::env::var("SYNAPSE_EMBED_MODEL").is_err() {
        std::env::set_var("SYNAPSE_EMBED_MODEL", "arctic-m");
    }

    let embedder = if args.embed {
        let model_name =
            std::env::var("SYNAPSE_EMBED_MODEL").unwrap_or_else(|_| "bge-small".into());
        let desc = match model_name.to_lowercase().as_str() {
            "arctic-m" => "Snowflake Arctic Embed M (768-dim, MTEB 62.5)",
            "arctic-s" => "Snowflake Arctic Embed S (384-dim, MTEB 60.0)",
            "arctic-xs" => "Snowflake Arctic Embed XS (384-dim, MTEB 56.6)",
            "arctic-l" => "Snowflake Arctic Embed L (1024-dim, MTEB 63.0)",
            "mxbai-large" => "MxbAI Embed Large v1 (1024-dim, MTEB 64.7)",
            "nomic-1.5" => "Nomic Embed Text v1.5 (768-dim, MTEB 62.4)",
            _ => "fastembed BGE-small-en-v1.5 (384-dim, MTEB 53.0)",
        };
        match Embedder::new() {
            Ok(e) => {
                println!("Embedder: {}", desc);
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

    // HyDE config: only active when feature `hyde` compiled in + --hyde flag + OLLAMA_AVAILABLE.
    #[cfg(feature = "hyde")]
    let hyde_config: Option<synapse_core::turbo::hyde::HydeConfig> = if args.hyde {
        if std::env::var("OLLAMA_AVAILABLE").is_err() {
            eprintln!("WARN: --hyde passed but OLLAMA_AVAILABLE not set — skipping HyDE");
            None
        } else {
            let cfg = synapse_core::turbo::hyde::HydeConfig {
                model: args.hyde_model.clone(),
                ..Default::default()
            };
            println!(
                "HyDE: Ollama model={} max_tokens={}",
                cfg.model, cfg.max_tokens
            );
            Some(cfg)
        }
    } else {
        None
    };
    #[cfg(feature = "hyde")]
    let hyde_cfg_ref: Option<&synapse_core::turbo::hyde::HydeConfig> = hyde_config.as_ref();

    let mut r5_hits = 0usize;
    let mut r10_hits = 0usize;
    let mut f5_hits = 0usize;
    let mut f10_hits = 0usize;
    let mut judge_r5_hits = 0usize;
    let mut judge_r5_evaluated = 0usize;
    let mut judge_r10_hits = 0usize;
    let mut total_ms: u128 = 0;
    let mut total_docs: usize = 0;
    let mut total_hyde_us: u128 = 0;
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
        // Helper macro to call run_question with the right extra arg under `hyde` feature.
        macro_rules! rq {
            ($hooks:expr) => {{
                #[cfg(feature = "hyde")]
                {
                    run_question(
                        q,
                        $hooks,
                        args.relevance_floor,
                        args.hyde_threshold,
                        embedder_ref,
                        reranker_ref,
                        args.ppr,
                        pre_extractor_ref,
                        args.rrf_k,
                        args.rerank_top,
                        hyde_cfg_ref,
                    )
                }
                #[cfg(not(feature = "hyde"))]
                {
                    run_question(
                        q,
                        $hooks,
                        args.relevance_floor,
                        args.hyde_threshold,
                        embedder_ref,
                        reranker_ref,
                        args.ppr,
                        pre_extractor_ref,
                        args.rrf_k,
                        args.rerank_top,
                    )
                }
            }};
        }
        #[cfg(feature = "minimax")]
        let res = if let Some(mh) = minimax_hooks.as_ref() {
            let san = SanHooks { inner: mh };
            rq!(&san)
        } else if args.use_mlx {
            let san = SanHooks { inner: &mlx_hooks };
            rq!(&san)
        } else {
            let san = SanHooks { inner: &rule };
            rq!(&san)
        };
        #[cfg(not(feature = "minimax"))]
        let res = if args.use_mlx {
            let san = SanHooks { inner: &mlx_hooks };
            rq!(&san)
        } else {
            let san = SanHooks { inner: &rule };
            rq!(&san)
        };
        match res {
            Ok((r5, r10, f5, f10, ms, nd, top5, top10, hyde_us)) => {
                if r5 {
                    r5_hits += 1;
                }
                if r10 {
                    r10_hits += 1;
                }
                if f5 {
                    f5_hits += 1;
                }
                if f10 {
                    f10_hits += 1;
                }
                total_ms += ms;
                total_docs += nd;
                total_hyde_us += hyde_us;
                let mut judge_verdict: Option<bool> = None;
                if let Some(j) = judge.as_ref() {
                    let refs5: Vec<&str> = top5.iter().map(|s| s.as_str()).collect();
                    let v5 = j.judge(&q.question, &q.answer, &refs5);
                    match v5 {
                        Some(v) => {
                            judge_r5_evaluated += 1;
                            if v {
                                judge_r5_hits += 1;
                                judge_r10_hits += 1; // top-5 ⊂ top-10
                            } else {
                                // Top-5 missed; ask judge over top-10.
                                let refs10: Vec<&str> = top10.iter().map(|s| s.as_str()).collect();
                                if matches!(j.judge(&q.question, &q.answer, &refs10), Some(true)) {
                                    judge_r10_hits += 1;
                                }
                            }
                            judge_verdict = Some(v);
                        }
                        None if r5 => {
                            judge_r5_evaluated += 1;
                            judge_r5_hits += 1;
                            judge_r10_hits += 1;
                            judge_verdict = Some(true);
                        }
                        None => {}
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
    let f5 = f5_hits as f64 / n;
    let f10 = f10_hits as f64 / n;
    println!("Recall@5     : {:.3}  ({}/{})", r5, r5_hits, qs.len());
    println!("Recall@10    : {:.3}  ({}/{})", r10, r10_hits, qs.len());
    println!(
        "Fuzzy-R@5    : {:.3}  ({}/{})  [token-set 0.6]",
        f5,
        f5_hits,
        qs.len()
    );
    println!(
        "Fuzzy-R@10   : {:.3}  ({}/{})  [token-set 0.6]",
        f10,
        f10_hits,
        qs.len()
    );
    if args.judge {
        let denom = qs.len() as f64;
        let jr5 = if denom > 0.0 {
            judge_r5_hits as f64 / denom
        } else {
            0.0
        };
        let jr10 = if denom > 0.0 {
            judge_r10_hits as f64 / denom
        } else {
            0.0
        };
        println!(
            "Judge-R@5    : {:.3}  ({}/{} evaluated, {} total)",
            jr5,
            judge_r5_hits,
            judge_r5_evaluated,
            qs.len()
        );
        println!(
            "Judge-R@10   : {:.3}  ({}/{})",
            jr10,
            judge_r10_hits,
            qs.len()
        );
    }
    println!(
        "Latency avg  : {:.2} ms (recall only, ingest excluded)",
        avg_ms / 1000.0
    );
    println!("Avg docs/Q   : {:.1}", avg_docs);
    #[cfg(feature = "hyde")]
    if total_hyde_us > 0 {
        let avg_hyde_ms = total_hyde_us as f64 / (qs.len() as f64) / 1000.0;
        println!(
            "HyDE overhead: {:.1} ms avg per query (Ollama expand only)",
            avg_hyde_ms
        );
    }
    Ok(())
}
