// jemalloc: replace system allocator — reduces fragmentation under alloc-heavy
// HNSW/ndarray workloads. Feature-gated so tests / cross-compile can opt out.
#[cfg(feature = "jemalloc")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

mod synx_io;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
#[cfg(any(feature = "static-ort", feature = "cross-linux"))]
use synapse_core::corpus::set_corpus_chunk_embedding;
#[cfg(any(feature = "static-ort", feature = "cross-linux"))]
use synapse_core::embed::Embedder;
#[cfg(feature = "sharding")]
use synapse_core::shard;
use synapse_core::{
    PutRequest, SearchMode, Store,
    corpus::{
        CorpusSourceKind, GoldQuestion, NewCorpusDocument, PromotionKind,
        bootstrap_eval_from_corpus, corpus_migrate, due_corpus_sync_sources, evaluate_rankings,
        evaluate_rankings_gate, gold_candidates_from_corpus, import_synapse_docs_to_corpus,
        ingest_fetched_document, ingest_pdf_bytes, ingest_rss_xml, ingest_web_html,
        ingest_youtube_transcript, mark_corpus_source_synced, put_corpus_document, queue_promotion,
        rank_gold_questions, ready_promotions, search_corpus, upsert_corpus_sync_source,
        verify_promotion, youtube_video_id,
    },
    federate::{Addr, Federation},
    fresh::{FreshMode, FreshOptions, build_fresh_report, render_fresh_context_xml},
    sign, snap,
    sota::{MemoryType, doc_memory_state, promote_doc_memory},
    temporal::{DateRange, format_timestamp, parse_temporal, parse_timestamp},
};
use synapse_learn::{ContextQueryLog, LearnStore};

type VerifyRow = (i64, String, Vec<u8>);
type FreshInput = (String, Option<PathBuf>, Option<String>);
type SearchBestEffortResult = (Vec<synapse_core::Hit>, String);
type FetchedUrl = (Option<String>, Vec<u8>);

#[cfg(feature = "network")]
fn fetch_url_bytes(url: &str) -> Result<FetchedUrl> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("synx/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("build HTTP client")?;
    let response = client
        .get(url)
        .send()
        .with_context(|| format!("fetch {url}"))?
        .error_for_status()
        .with_context(|| format!("fetch {url}"))?;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let bytes = response.bytes().context("read response body")?.to_vec();
    Ok((content_type, bytes))
}

#[cfg(not(feature = "network"))]
fn fetch_url_bytes(_url: &str) -> Result<FetchedUrl> {
    Err(anyhow::anyhow!(
        "network fetch is not included in this portable build; download the source yourself and use local corpus ingest"
    ))
}

#[cfg(any(feature = "static-ort", feature = "cross-linux"))]
fn semantic_embedding(file: &std::path::Path, text: &str) -> Result<Vec<f32>> {
    let embedder = Embedder::new_with_cache::<std::path::PathBuf>(
        file.parent().map(|parent| parent.join(".emb-cache")),
    )
    .context("embedder init")?;
    embedder.embed_one(text).map_err(Into::into)
}

#[cfg(not(any(feature = "static-ort", feature = "cross-linux")))]
fn semantic_embedding(_file: &std::path::Path, _text: &str) -> Result<Vec<f32>> {
    Err(anyhow::anyhow!(
        "semantic embeddings are not included in this portable build; use lexical/context commands or install a semantic build"
    ))
}

fn optional_document_embedding(
    _file: &std::path::Path,
    _text: &str,
    disabled: bool,
) -> Result<Option<Vec<f32>>> {
    if disabled {
        return Ok(None);
    }
    #[cfg(any(feature = "static-ort", feature = "cross-linux"))]
    {
        return semantic_embedding(_file, _text).map(Some);
    }
    #[cfg(not(any(feature = "static-ort", feature = "cross-linux")))]
    {
        eprintln!(
            "warning: portable build stores this memory without an embedding; lexical and cited context retrieval remain available"
        );
        Ok(None)
    }
}

#[derive(Parser)]
#[command(
    name = "synx",
    version,
    about = "Synapse Agent Memory — local-first memory for AI coding agents"
)]
struct Cli {
    #[arg(short = 'f', long, default_value = ".synapse/brain.db", global = true)]
    file: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Initialize a new memory file
    Init,
    /// Append a doc from stdin (or --text)
    Put {
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        uri: Option<String>,
        #[arg(long)]
        text: Option<String>,
        /// Human-readable source/citation for this memory.
        #[arg(long)]
        source: Option<String>,
        /// Freshness date for this memory (YYYY-MM-DD or RFC3339).
        #[arg(long)]
        updated: Option<String>,
        /// Memory kind, e.g. decision|fact|bugfix|benchmark|research|note.
        #[arg(long)]
        kind: Option<String>,
        /// Trust/status marker, e.g. active|archived|verified|stale.
        #[arg(long)]
        status: Option<String>,
        /// Extra JSON metadata object merged into the stored meta column.
        #[arg(long)]
        meta: Option<String>,
        #[arg(long, default_value_t = false)]
        no_embed: bool,
        /// Path to Ed25519 signing key (32-byte raw file)
        #[arg(long)]
        sign: Option<PathBuf>,
    },
    /// Verify Ed25519 signature of a doc by id
    Verify {
        id: i64,
        /// Path to verifying key (32-byte raw file)
        #[arg(long)]
        vk: PathBuf,
    },
    /// Generate an Ed25519 keypair
    Keygen {
        /// Output secret key path
        #[arg(long, default_value = "synapse.sk")]
        sk: PathBuf,
        /// Output public key path
        #[arg(long, default_value = "synapse.vk")]
        vk: PathBuf,
    },
    /// Export signed .brainpack
    SnapSigned {
        out: PathBuf,
        #[arg(long, default_value_t = 3)]
        level: i32,
        #[arg(long)]
        sk: PathBuf,
    },
    /// Lexical FTS5 search
    Find {
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Vector kNN search
    Vec {
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Hybrid (RRF fusion) search
    Hybrid {
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: usize,
        /// Exact-rerank guarantee: full brute-force cosine after RRF (R@N=1.0, +1-2ms)
        #[arg(long, default_value_t = false)]
        guarantee: bool,
    },
    /// Compile a bounded, cited context pack for an agent task
    Context {
        query: String,
        /// coding|research|decision|debug|daily|auto
        #[arg(long, default_value = "auto")]
        mode: String,
        #[arg(long, default_value_t = 12)]
        limit: usize,
        /// Character budget for retrieved snippets
        #[arg(long, default_value_t = 2400)]
        budget: usize,
        /// Emit machine-readable JSON instead of Markdown
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Store typed memory with metadata for better future context
    Remember {
        text: String,
        /// decision|fact|preference|bugfix|benchmark|command|session|adr|research|note
        #[arg(long, default_value = "decision")]
        kind: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        uri: Option<String>,
        /// stable|slow|fast|volatile
        #[arg(long, default_value = "stable")]
        freshness: String,
        /// high|medium|low
        #[arg(long, default_value = "high")]
        confidence: String,
        /// critical|high|normal|low — bounded ranking signal, not absolute truth.
        #[arg(long, default_value = "normal")]
        priority: String,
        /// When the event happened: YYYY-MM-DD or RFC3339.
        #[arg(long)]
        occurred_at: Option<String>,
        /// Previous docs.id whose active memory this one replaces.
        #[arg(long)]
        supersedes: Option<i64>,
        #[arg(long, default_value_t = false)]
        no_embed: bool,
    },
    /// Health check plus safe self-healing hints/repairs
    Doctor {
        #[arg(long, default_value_t = false)]
        fix: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Search with automatic fallback: hybrid → lexical → recent timeline
    Fallback {
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Build a repo startup brief: git state, source docs, tests, memories, freshness
    Prime {
        /// Repository or project directory to inspect.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// coding|research|decision|debug|daily|auto
        #[arg(long, default_value = "auto")]
        mode: String,
        /// Number of recent/relevant memories to include.
        #[arg(long, default_value_t = 8)]
        limit: usize,
        /// Emit machine-readable JSON instead of Markdown.
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Stats
    Stats,
    /// Build version-pinned package/API context from local manifests and lockfiles
    FreshContext {
        /// Prompt/task text. If omitted, stdin is read; hook JSON with prompt/cwd is accepted.
        #[arg(long)]
        prompt: Option<String>,
        /// Project cwd to scan for manifests/lockfiles.
        #[arg(long)]
        cwd: Option<PathBuf>,
        /// Project name for rendered context.
        #[arg(long)]
        project: Option<String>,
        /// Context mode: prompt or session.
        #[arg(long, default_value = "prompt")]
        mode: String,
        /// Emit JSON FreshReport instead of the XML context block.
        #[arg(long, default_value_t = false)]
        json: bool,
        /// Disable registry latest lookups; still emits local/resolved docs.
        #[arg(long, default_value_t = false)]
        no_registry: bool,
        /// Override number of deps checked against registries.
        #[arg(long)]
        max_registry: Option<usize>,
    },
    /// Export to .brainpack
    Snap {
        out: PathBuf,
        #[arg(long, default_value_t = 3)]
        level: i32,
    },
    /// Import a .brainpack into this file
    /// (extension doesn't matter, content does — .synx/.synapse/.brainpack/.bp all accepted)
    Restore { pack: PathBuf },
    /// Merge two brainpacks by URI-matching docs, CRDT-merging meta_crdt per doc
    /// (extension doesn't matter, content does — .synx/.synapse/.brainpack/.bp all accepted)
    Merge {
        file_a: PathBuf,
        file_b: PathBuf,
        #[arg(short = 'o', long)]
        out: PathBuf,
        #[arg(long, default_value_t = 3)]
        level: i32,
    },
    /// Merge a peer snapshot into the current brain file (CRDT, offline-safe).
    /// Equivalent to: synx merge <current-brain-snap> <peer> --out merged.brainpack
    MergeSnap {
        peer: PathBuf,
        #[arg(short = 'o', long, default_value = "merged.brainpack")]
        out: PathBuf,
        #[arg(long, default_value_t = 3)]
        level: i32,
    },
    /// Sign a doc by id — writes/updates ed25519 signature on the stored doc.
    Sign {
        id: i64,
        /// Path to Ed25519 signing key (32-byte raw file)
        #[arg(long, default_value = "synapse.sk")]
        sk: PathBuf,
    },
    /// Federation: peer-to-peer CRDT sync
    Federate {
        #[command(subcommand)]
        action: FederateCmd,
    },
    /// IVF shard operations
    #[cfg(feature = "sharding")]
    Shard {
        #[command(subcommand)]
        action: ShardCmd,
    },
    /// Self-learning operations
    Learn {
        #[command(subcommand)]
        action: LearnCmd,
    },
    /// Record explicit pass/fail feedback for a generated context pack
    Feedback {
        query_id: String,
        /// Accepted docs.id (compatible with the v1 positional form).
        accepted_doc_id: Option<i64>,
        /// pass|fail
        #[arg(long, default_value = "pass")]
        gate: String,
        /// Comma-separated docs.id values actually used by the agent.
        #[arg(long)]
        used: Option<String>,
        #[arg(long, default_value = "default")]
        shard_id: String,
    },
    /// Export db to portable .synx container (alias for snap with optional encryption)
    Backup {
        /// Source db path (defaults to --file)
        #[arg(long)]
        db: Option<PathBuf>,
        /// Output .synx file
        out: PathBuf,
        #[arg(long, default_value_t = 3)]
        level: i32,
        /// Encrypt with age passphrase
        #[arg(long)]
        encrypt: bool,
        /// Passphrase for encryption
        #[arg(long)]
        passphrase: Option<String>,
    },
    /// Import .synx container into a db (idempotent: skips docs already present by blake3)
    DbRestore {
        /// Input .synx file
        pack: PathBuf,
        /// Target db path (defaults to --file)
        #[arg(long)]
        db: Option<PathBuf>,
        /// Passphrase if pack is encrypted
        #[arg(long)]
        passphrase: Option<String>,
    },
    /// Integrity check: verify blake3 of every doc
    DbVerify {
        /// db path (defaults to --file)
        #[arg(long)]
        db: Option<PathBuf>,
    },
    /// Safely repair FTS5 after a verified brainpack backup (vectors unchanged)
    DbRepair {
        /// db path (defaults to --file)
        #[arg(long)]
        db: Option<PathBuf>,
    },
    /// Import docs from file (auto-detect format by extension)
    /// Formats: .csv .tsv .jsonl .ndjson .json .db .sqlite .sqlite3 .synx .brainpack
    /// Optional (--features import-parquet): .parquet
    Import {
        /// Source file path
        src: PathBuf,
        /// Force format (csv|tsv|jsonl|json|sqlite|synx|parquet)
        #[arg(long)]
        format: Option<String>,
    },
    /// Export docs to file (auto-detect format by extension)
    /// Formats: .synx .brainpack .csv .tsv .jsonl .db .sqlite .sqlite3
    /// Optional (--features import-parquet): .parquet
    Export {
        /// Destination file path
        dst: PathBuf,
        /// Force format
        #[arg(long)]
        format: Option<String>,
    },
    /// Convert between formats: import src then export to dst
    Convert {
        src: PathBuf,
        dst: PathBuf,
        /// Force source format
        #[arg(long)]
        from: Option<String>,
        /// Force destination format
        #[arg(long)]
        to: Option<String>,
    },
    /// Graph operations (PageRank, communities, traversal, Cypher)
    Graph {
        #[command(subcommand)]
        action: GraphCmd,
    },
    /// One-shot grounding: hybrid seeds → PageRank → traverse → JSON bundle
    Ground {
        query: String,
        #[arg(long, default_value_t = 20)]
        k: usize,
        #[arg(long, default_value_t = 2)]
        depth: usize,
        #[arg(long, default_value_t = 0.5)]
        alpha: f64,
        #[arg(long, default_value_t = 10)]
        iters: usize,
    },
    /// Raw corpus sidecar: ingest/search/eval before verified memory promotion
    Corpus {
        #[command(subcommand)]
        action: CorpusCmd,
    },
}

#[derive(Subcommand)]
enum CorpusCmd {
    /// Add raw text to the corpus sidecar. Text comes from --text or stdin.
    AddText {
        #[arg(long, default_value = "text")]
        kind: String,
        #[arg(long)]
        source_uri: String,
        #[arg(long)]
        external_id: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        text: Option<String>,
        #[arg(long)]
        published_ts: Option<i64>,
        /// Embed inserted chunks so vector+RRF retrieval can use them.
        #[arg(long, default_value_t = false)]
        embed: bool,
    },
    /// Add RSS XML to the corpus sidecar. XML comes from --path or stdin.
    AddRss {
        #[arg(long)]
        source_uri: String,
        #[arg(long)]
        path: Option<PathBuf>,
        /// Embed inserted/existing feed documents so vector+RRF retrieval can use them.
        #[arg(long, default_value_t = false)]
        embed: bool,
    },
    /// Add a YouTube transcript (plain text, VTT, or SRT) to the corpus sidecar.
    AddYoutube {
        #[arg(long)]
        video_id: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        source_uri: Option<String>,
        #[arg(long)]
        path: Option<PathBuf>,
        /// Embed inserted transcript chunks so vector+RRF retrieval can use them.
        #[arg(long, default_value_t = false)]
        embed: bool,
    },
    /// Fetch YouTube subtitles/transcript with yt-dlp and ingest them.
    FetchYoutube {
        url: String,
        #[arg(long)]
        video_id: Option<String>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, default_value = "en.*,en")]
        lang: String,
        #[arg(long, default_value = "yt-dlp")]
        yt_dlp: PathBuf,
        /// Embed inserted transcript chunks so vector+RRF retrieval can use them.
        #[arg(long, default_value_t = false)]
        embed: bool,
    },
    /// Add an HTML web page to the corpus sidecar. HTML comes from --path or stdin.
    AddWeb {
        #[arg(long)]
        source_uri: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        path: Option<PathBuf>,
        /// Embed inserted page chunks so vector+RRF retrieval can use them.
        #[arg(long, default_value_t = false)]
        embed: bool,
    },
    /// Add a PDF to the corpus sidecar. Bytes come from --path or stdin.
    AddPdf {
        #[arg(long)]
        source_uri: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        path: Option<PathBuf>,
        /// Embed inserted PDF chunks so vector+RRF retrieval can use them.
        #[arg(long, default_value_t = false)]
        embed: bool,
    },
    /// Fetch a URL and route by Content-Type/URI into RSS, PDF, HTML, or text ingest.
    FetchUrl {
        url: String,
        #[arg(long)]
        title: Option<String>,
        /// Override the HTTP Content-Type before routing.
        #[arg(long)]
        content_type: Option<String>,
        /// Embed inserted chunks so vector+RRF retrieval can use them.
        #[arg(long, default_value_t = false)]
        embed: bool,
    },
    /// Register or update a URL source for recurring `sync-due`.
    WatchUrl {
        url: String,
        #[arg(long, default_value = "web")]
        kind: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, default_value_t = 86_400)]
        every_secs: i64,
    },
    /// Fetch and ingest all watched URL sources that are due.
    SyncDue {
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long, default_value_t = false)]
        embed: bool,
    },
    /// Import existing Synapse Agent Memory docs into the corpus sidecar for real-usage evals.
    ImportSynapse {
        #[arg(long, default_value_t = 100)]
        limit: usize,
        /// Embed imported chunks so vector+RRF retrieval can use them.
        #[arg(long, default_value_t = false)]
        embed: bool,
    },
    /// Search the raw corpus sidecar. Default is FTS5; --embed adds vector leg.
    Search {
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: usize,
        #[arg(long, default_value_t = false)]
        embed: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Queue a corpus chunk as a possible durable fact/decision.
    Promote {
        chunk_id: i64,
        #[arg(long, default_value = "decision")]
        kind: String,
        #[arg(long)]
        rationale: String,
    },
    /// Verify a queued promotion. Only verified promotions become ready.
    Verify {
        promotion_id: i64,
        #[arg(long)]
        verifier: String,
    },
    /// List verified promotion IDs ready for `synx remember`/`synx put`.
    Ready,
    /// Export corpus-grounded gold-question candidates for manual eval curation.
    GoldCandidates {
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Snapshot current corpus rankings for a gold set before retrieval changes.
    BaselineRankings {
        gold_json: PathBuf,
        #[arg(long, default_value_t = 5)]
        limit: usize,
        #[arg(long, default_value_t = false)]
        embed: bool,
    },
    /// Generate real-usage eval artifacts and enforce the min-gold gate.
    BootstrapEval {
        /// Candidate/gold question count target. Use 50-100 for the real gate.
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long, default_value_t = 50)]
        min_gold: usize,
        #[arg(long, default_value_t = 5)]
        rank_limit: usize,
        /// First mirror existing Synapse Agent Memory docs into corpus before bootstrapping.
        #[arg(long, default_value_t = false)]
        import_synapse: bool,
        /// Write editable candidates with title/source/preview.
        #[arg(long)]
        candidates_json: Option<PathBuf>,
        /// Write machine gold questions for eval/eval-gate.
        #[arg(long)]
        gold_json: Option<PathBuf>,
        /// Write current retrieval rankings as the baseline.
        #[arg(long)]
        baseline_rankings_json: Option<PathBuf>,
    },
    /// Evaluate corpus retrieval against a JSON gold set.
    Eval {
        gold_json: PathBuf,
        #[arg(long, default_value_t = 5)]
        limit: usize,
        #[arg(long, default_value_t = false)]
        embed: bool,
    },
    /// Evaluate current corpus against baseline rankings and fail unless the gate improves.
    EvalGate {
        gold_json: PathBuf,
        baseline_rankings_json: PathBuf,
        #[arg(long, default_value_t = 5)]
        limit: usize,
        #[arg(long, default_value_t = 50)]
        min_gold: usize,
        #[arg(long, default_value_t = false)]
        embed: bool,
    },
}

#[derive(Subcommand)]
enum GraphCmd {
    /// Insert/replace edge: from -[rel:weight]-> to
    Relate {
        from: i64,
        to: i64,
        rel: String,
        #[arg(long, default_value_t = 1.0)]
        weight: f64,
    },
    /// Top-N nodes by PageRank
    Pagerank {
        #[arg(long, default_value_t = 20)]
        n: usize,
        #[arg(long, default_value_t = 0.85)]
        damping: f64,
        #[arg(long, default_value_t = 20)]
        iters: usize,
    },
    /// Personalized PageRank seeded by JSON map {id: weight}
    Ppr {
        /// JSON dict of seed_id → score, e.g. '{"42":1.0,"7":0.5}'
        seeds_json: String,
        #[arg(long, default_value_t = 0.5)]
        alpha: f64,
        #[arg(long, default_value_t = 10)]
        iters: usize,
        #[arg(long, default_value_t = 30)]
        limit: usize,
    },
    /// Detect communities via label-propagation
    Communities {
        #[arg(long, default_value_t = 20)]
        max_iters: usize,
        #[arg(long, default_value_t = 20)]
        top_n: usize,
    },
    /// Direct neighbors of a node
    Neighbors {
        node_id: i64,
        #[arg(long, default_value_t = 50)]
        top_k: usize,
        #[arg(long)]
        rel: Option<String>,
    },
    /// Traverse outward from start node
    Traverse {
        start_id: i64,
        #[arg(long, default_value_t = 3)]
        depth: usize,
        #[arg(long, default_value_t = 10)]
        top_k_per_hop: usize,
        #[arg(long, default_value_t = 0.7)]
        decay: f64,
    },
    /// Shortest path (Dijkstra) between two nodes
    Path {
        from: i64,
        to: i64,
        #[arg(long, default_value_t = 5)]
        max_depth: usize,
    },
    /// Edge count
    Count,
}

#[derive(Subcommand)]
enum LearnCmd {
    /// Show learning stats
    Status,
    /// Run near-dup consolidation
    Consolidate,
    /// Check embedding drift
    DriftCheck,
    /// Update calibration from feedback log
    Calibrate,
}

#[derive(Subcommand)]
enum FederateCmd {
    /// Add a peer address (tcp:host:port or unix:/path)
    Add {
        addr: String,
        /// Ed25519 signing key for this node
        #[arg(long, default_value = "synapse.sk")]
        sk: PathBuf,
    },
    /// Sync all known peers
    Sync {
        #[arg(long, default_value = "synapse.sk")]
        sk: PathBuf,
        /// Peer addresses to sync (tcp:host:port or unix:/path)
        peers: Vec<String>,
    },
    /// List configured peers
    Peers {
        #[arg(long, default_value = "synapse.sk")]
        sk: PathBuf,
        /// Peer addresses
        peers: Vec<String>,
    },
}

#[derive(Subcommand)]
#[cfg(feature = "sharding")]
enum ShardCmd {
    /// Split a brain.db into N shards (k-means on embeddings)
    Split {
        brain: PathBuf,
        #[arg(short = 'o', long)]
        out_dir: PathBuf,
        #[arg(long)]
        shards: Option<usize>,
    },
    /// Query a shard manifest (bloom prefilter → centroid-nearest → fan-out → RRF)
    Query {
        manifest: PathBuf,
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
}

/// Resolve the brain path so commands work from ANY cwd (agents/MCP run from
/// arbitrary dirs). The clap default `.synapse/brain.db` is cwd-relative; if it
/// doesn't exist in the cwd but a `$HOME/.synapse/brain.db` does, use the home
/// brain. Absolute paths and existing cwd-relative project brains are untouched,
/// and an explicit `-f` always wins. Net: no behaviour change when a local brain
/// exists; fixes silent "empty db" when run outside `~`.
fn resolve_db_path(p: std::path::PathBuf) -> std::path::PathBuf {
    if p.is_absolute() || p.exists() {
        return p;
    }
    if let Some(home) = std::env::var_os("HOME") {
        let h = std::path::PathBuf::from(home).join(&p);
        if h.exists() {
            return h;
        }
    }
    p
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let mut cli = Cli::parse();
    cli.file = resolve_db_path(cli.file);
    if let Some(p) = cli.file.parent() {
        std::fs::create_dir_all(p).ok();
    }
    match cli.cmd {
        Cmd::Init => {
            Store::open(&cli.file)?;
            println!("ok init {}", cli.file.display());
        }
        Cmd::Put {
            title,
            uri,
            text,
            source,
            updated,
            kind,
            status,
            meta,
            no_embed,
            sign: sign_path,
        } => {
            let body = match text {
                Some(t) => t,
                None => {
                    let mut s = String::new();
                    std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
                    s.trim().to_string()
                }
            };
            anyhow::ensure!(!body.is_empty(), "empty text");
            let mut store = Store::open(&cli.file)?;
            let embedding = optional_document_embedding(&cli.file, &body, no_embed)?;
            let meta = build_put_meta(source, updated, kind, status, meta)?;
            let req = PutRequest {
                title,
                uri,
                text: body,
                meta,
                embedding,
            };
            let id = if let Some(sk_path) = sign_path {
                let sk = sign::load_signing_key(&sk_path).context("load signing key")?;
                store.put_signed(&req, Some(&sk))?
            } else {
                store.put(&req)?
            };
            println!("{}", id);
        }
        Cmd::Verify { id, vk } => {
            let store = Store::open(&cli.file)?;
            let vk = sign::load_verifying_key(&vk).context("load verifying key")?;
            store.verify(id, &vk)?;
            println!("ok verified id={}", id);
        }
        Cmd::Keygen { sk, vk } => {
            sign::keygen(&sk, &vk).context("keygen")?;
            println!("ok sk={} vk={}", sk.display(), vk.display());
        }
        Cmd::SnapSigned { out, level, sk } => {
            let signing_key = sign::load_signing_key(&sk).context("load signing key")?;
            snap::export_signed(&cli.file, &out, level, &signing_key)?;
            println!("ok snap-signed {}", out.display());
        }
        Cmd::Find { query, limit } => {
            let store = Store::open(&cli.file)?;
            let hits = store.search(&query, SearchMode::Lex, None, limit)?;
            print_hits(&hits);
        }
        Cmd::Vec { query, limit } => {
            let store = Store::open(&cli.file)?;
            let q = semantic_embedding(&cli.file, &query)?;
            let hits = store.search("", SearchMode::Vec, Some(&q), limit)?;
            print_hits(&hits);
        }
        Cmd::Hybrid {
            query,
            limit,
            guarantee,
        } => {
            let store = Store::open(&cli.file)?;
            let q = semantic_embedding(&cli.file, &query)?;
            let hits = if guarantee {
                // Two-stage: hybrid RRF for candidate expansion, then exact brute-force vec
                let candidates = store.search(&query, SearchMode::Hybrid, Some(&q), limit * 10)?;
                // Re-rank the candidates via exact cosine
                let _ = candidates; // candidates already filtered by RRF; now exact vec over full corpus
                store.search_vec_exact(&q, limit)?
            } else {
                store.search(&query, SearchMode::Hybrid, Some(&q), limit)?
            };
            print_hits(&hits);
        }
        Cmd::Context {
            query,
            mode,
            limit,
            budget,
            json,
        } => {
            anyhow::ensure!(limit > 0, "--limit must be greater than zero");
            anyhow::ensure!(budget > 0, "--budget must be greater than zero");
            let store = Store::open(&cli.file)?;
            let learn_path = cli.file.with_extension("learn.db");
            let lstore = LearnStore::open(&learn_path).ok();
            let temporal_range = parse_temporal(&query, now_secs());
            let retrieval_query = temporal_range
                .map(|_| temporal_retrieval_query(&query))
                .unwrap_or_else(|| query.clone());
            let retrieval_limit = limit.saturating_mul(4).max(limit);
            let (hits, mut route) =
                search_best_effort(&store, &cli.file, &retrieval_query, retrieval_limit)?;
            let (mut ranked, mut diagnostics) =
                rank_context_hits_in_range(&store, lstore.as_ref(), hits, temporal_range)?;
            if retrieval_query != query {
                diagnostics.retrieval_query = Some(retrieval_query);
            }
            if ranked.is_empty()
                && let Some(range) = temporal_range
            {
                let timeline_hits = store
                    .timeline_between(range.lo, range.hi, retrieval_limit)?
                    .into_iter()
                    .map(doc_to_hit)
                    .collect();
                let (fallback_ranked, fallback_diagnostics) = rank_context_hits_in_range(
                    &store,
                    lstore.as_ref(),
                    timeline_hits,
                    Some(range),
                )?;
                ranked = fallback_ranked;
                diagnostics.merge(fallback_diagnostics);
                route = "event_timeline".to_string();
            }
            ranked.truncate(limit);
            let (packed, used_chars) = bounded_context_hits(&ranked, budget);
            let context_id = context_id(&query, &mode, &packed);
            if let Some(ls) = lstore.as_ref() {
                let ids: Vec<i64> = packed.iter().map(|h| h.id).collect();
                let scores = normalized_context_scores(&packed);
                let kinds: Vec<String> = packed.iter().map(context_hit_kind).collect();
                ls.log_context_query(&ContextQueryLog {
                    context_id: &context_id,
                    ts: now_secs(),
                    query: &query,
                    mode: &mode,
                    route: &route,
                    doc_ids: &ids,
                    scores: &scores,
                    kinds: &kinds,
                    budget_chars: budget,
                    used_chars,
                })
                .context("log context pack for feedback")?;
            }
            let output = ContextOutput {
                context_id: &context_id,
                query: &query,
                mode: &mode,
                budget,
                used_chars,
                route: &route,
                hits: &packed,
                diagnostics: &diagnostics,
            };
            if json {
                print_context_json(&output)?;
            } else {
                print_context_pack(&output);
            }
        }
        Cmd::Remember {
            text,
            kind,
            title,
            uri,
            freshness,
            confidence,
            priority,
            occurred_at,
            supersedes,
            no_embed,
        } => {
            anyhow::ensure!(!text.trim().is_empty(), "empty text");
            let mut store = Store::open(&cli.file)?;
            let normalized_kind = normalize_kind(&kind)
                .with_context(|| format!("unsupported memory kind: {kind}"))?;
            let freshness = normalize_freshness(&freshness)
                .with_context(|| format!("unsupported freshness: {freshness}"))?;
            let confidence_score = confidence_score(&confidence)
                .with_context(|| format!("unsupported confidence: {confidence}"))?;
            let priority = normalize_priority(&priority)
                .with_context(|| format!("unsupported priority: {priority}"))?;
            let occurred_ts = occurred_at
                .as_deref()
                .map(|value| {
                    parse_timestamp(value)
                        .with_context(|| format!("invalid --occurred-at value: {value}"))
                })
                .transpose()?;
            let occurred_at = occurred_ts.and_then(format_timestamp);
            let embedding = optional_document_embedding(&cli.file, &text, no_embed)?;
            let captured_at = now_ms();
            let req = PutRequest {
                title: title.or_else(|| Some(auto_title(&normalized_kind, &text))),
                uri,
                text: text.clone(),
                meta: Some(serde_json::json!({
                    "kind": normalized_kind,
                    "freshness": freshness,
                    "confidence": confidence,
                    "confidence_score": confidence_score,
                    "priority": priority,
                    "observed_at": captured_at,
                    "occurred_at": occurred_at,
                    "occurred_ts": occurred_ts,
                    "source": "synx remember",
                    "chunker": "synx-cli-v1.1"
                })),
                embedding,
            };
            let id = store.put(&req)?;
            merge_remember_metadata(
                &store,
                id,
                &normalized_kind,
                &freshness,
                &confidence,
                confidence_score,
                &priority,
                captured_at,
                occurred_at.as_deref(),
                occurred_ts,
            )?;
            let memory_id = promote_doc_memory(
                &store.conn,
                id,
                memory_type_for_kind(&normalized_kind),
                confidence_score,
                occurred_at.as_deref(),
                supersedes,
            )?;
            println!(
                "ok remembered id={} memory_id={} kind={} priority={}{}",
                id,
                memory_id,
                normalized_kind,
                priority,
                supersedes
                    .map(|old| format!(" supersedes={old}"))
                    .unwrap_or_default()
            );
        }
        Cmd::Doctor { fix, json } => {
            let store = Store::open(&cli.file)?;
            let repair = if fix {
                Some(safe_repair(&store, &cli.file)?)
            } else {
                None
            };
            let mut report = doctor_report(&store, &cli.file)?;
            report.repair = repair;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_doctor_report(&report);
            }
        }
        Cmd::Fallback { query, limit } => {
            let store = Store::open(&cli.file)?;
            let (hits, _) = search_best_effort(&store, &cli.file, &query, limit)?;
            print_hits(&hits);
        }
        Cmd::Prime {
            path,
            mode,
            limit,
            json,
        } => {
            let store = Store::open(&cli.file).ok();
            let report = build_prime_report(&path, store.as_ref(), &cli.file, &mode, limit)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_prime_report(&report);
            }
        }
        Cmd::Stats => {
            let store = Store::open(&cli.file)?;
            let s = store.stats()?;
            println!("{}", serde_json::to_string_pretty(&s)?);
        }
        Cmd::FreshContext {
            prompt,
            cwd,
            project,
            mode,
            json,
            no_registry,
            max_registry,
        } => {
            let raw = match prompt {
                Some(p) => p,
                None => {
                    let mut s = String::new();
                    std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
                    s
                }
            };
            let (input_prompt, input_cwd, input_project) = parse_fresh_input(&raw);
            let mode = FreshMode::parse(&mode);
            let mut opts = FreshOptions::from_env(mode);
            if no_registry {
                opts.max_registry = 0;
            }
            if let Some(n) = max_registry {
                opts.max_registry = n;
            }
            if let Some(report) = build_fresh_report(
                &input_prompt,
                mode,
                cwd.or(input_cwd).as_deref(),
                project.or(input_project).as_deref(),
                &opts,
            )? {
                if json {
                    println!("{}", serde_json::to_string_pretty(&report)?);
                } else {
                    println!("{}", render_fresh_context_xml(&report));
                }
            }
        }
        Cmd::Snap { out, level } => {
            snap::export(&cli.file, &out, level)?;
            println!("ok snap {}", out.display());
        }
        Cmd::Restore { pack } => {
            snap::import(&pack, &cli.file)?;
            println!("ok restore {}", cli.file.display());
        }
        Cmd::Merge {
            file_a,
            file_b,
            out,
            level,
        } => {
            snap::merge_packs(&file_a, &file_b, &out, level)?;
            println!("ok merge {}", out.display());
        }
        Cmd::MergeSnap { peer, out, level } => {
            // Export current brain as a temp snap, then merge with peer.
            let tmp =
                std::env::temp_dir().join(format!("synapse-snap-{}.brainpack", std::process::id()));
            snap::export(&cli.file, &tmp, level)?;
            snap::merge_packs(&tmp, &peer, &out, level)?;
            let _ = std::fs::remove_file(&tmp);
            println!("ok merge-snap {}", out.display());
        }
        Cmd::Sign { id, sk } => {
            let store = Store::open(&cli.file)?;
            let doc = store.get(id)?;
            let signing_key = sign::load_signing_key(&sk).context("load signing key")?;
            // Sign blake3(text) — same algorithm as Store::verify uses
            let hash = blake3::hash(doc.text.as_bytes());
            let sig = sign::sign_bytes(&signing_key, hash.as_bytes());
            // Write signature into the sig column of the existing row
            store
                .conn
                .execute(
                    "UPDATE docs SET sig = ?1 WHERE id = ?2",
                    rusqlite::params![sig.as_ref(), id],
                )
                .context("update sig")?;
            println!("ok signed id={}", id);
        }
        #[cfg(feature = "sharding")]
        Cmd::Shard { action } => match action {
            ShardCmd::Split {
                brain,
                out_dir,
                shards,
            } => {
                let manifest = shard::split(&brain, &out_dir, shards)?;
                let manifest_path = out_dir.join("brain.shards.toml");
                manifest.save(&manifest_path)?;
                println!(
                    "ok split into {} shards → {}",
                    manifest.shards.len(),
                    manifest_path.display()
                );
            }
            ShardCmd::Query {
                manifest,
                query,
                limit,
            } => {
                let manager = shard::ShardManager::open(manifest)?;
                let q_vec = semantic_embedding(&cli.file, &query)?;
                let q_arr: [f32; synapse_core::types::EMBED_DIM] = q_vec
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("embedding dim mismatch"))?;
                let hits = manager.query(&query, &q_arr, SearchMode::Hybrid, limit)?;
                print_hits(&hits);
            }
        },
        Cmd::Federate { action } => match action {
            FederateCmd::Add { addr, sk } => {
                let sk = sign::load_signing_key(&sk).context("load signing key")?;
                let fed = Federation::new(sk);
                let peer: Addr = addr.parse().context("parse peer addr")?;
                fed.add_peer(peer.clone());
                println!("ok added peer {}", peer);
            }
            FederateCmd::Sync { sk, peers } => {
                let sk = sign::load_signing_key(&sk).context("load signing key")?;
                let fed = Federation::new(sk);
                for p in &peers {
                    let peer: Addr = p.parse().context("parse peer addr")?;
                    fed.add_peer(peer);
                }
                fed.sync_all()?;
                println!("ok synced {} peers", peers.len());
            }
            FederateCmd::Peers { sk, peers } => {
                let sk = sign::load_signing_key(&sk).context("load signing key")?;
                let fed = Federation::new(sk);
                for p in &peers {
                    let peer: Addr = p.parse().context("parse peer addr")?;
                    fed.add_peer(peer);
                }
                for p in fed.peers() {
                    println!("{}", p);
                }
            }
        },
        Cmd::Learn { action } => {
            let learn_path = cli.file.with_extension("learn.db");
            let lstore = LearnStore::open(&learn_path)?;
            match action {
                LearnCmd::Status => {
                    let bandit_count: i64 = lstore
                        .conn
                        .query_row("SELECT COUNT(*) FROM learn_bandit", [], |r| r.get(0))
                        .unwrap_or(0);
                    let fb_count: i64 = lstore
                        .conn
                        .query_row("SELECT COUNT(*) FROM feedback", [], |r| r.get(0))
                        .unwrap_or(0);
                    let context_count: i64 = lstore
                        .conn
                        .query_row("SELECT COUNT(*) FROM context_query_log", [], |r| r.get(0))
                        .unwrap_or(0);
                    let rewarded_count: i64 = lstore
                        .conn
                        .query_row(
                            "SELECT COUNT(*) FROM context_query_log WHERE reward IS NOT NULL",
                            [],
                            |r| r.get(0),
                        )
                        .unwrap_or(0);
                    let calibration_samples: i64 = lstore
                        .conn
                        .query_row(
                            "SELECT COALESCE(SUM(samples),0) FROM learn_calibration",
                            [],
                            |r| r.get(0),
                        )
                        .unwrap_or(0);
                    println!(
                        "bandit_shards={} feedback_entries={} context_packs={} rewarded_packs={} calibration_samples={}",
                        bandit_count, fb_count, context_count, rewarded_count, calibration_samples
                    );
                }
                LearnCmd::Consolidate => {
                    let store = Store::open(&cli.file)?;
                    let report = synapse_learn::consolidate::run_consolidate(&store.conn)?;
                    println!(
                        "pairs_found={} merged={}",
                        report.pairs_found, report.merged
                    );
                }
                LearnCmd::DriftCheck => {
                    println!("drift-check: requires embedded model — run with --feature embed");
                }
                LearnCmd::Calibrate => {
                    let updated = synapse_learn::calibrate::update_calibration(&lstore)?;
                    println!("calibration updated buckets={}", updated);
                }
            }
        }
        Cmd::Feedback {
            query_id,
            accepted_doc_id,
            gate,
            used,
            shard_id,
        } => {
            let learn_path = cli.file.with_extension("learn.db");
            let lstore = LearnStore::open(&learn_path)?;
            let passed = match gate.trim().to_ascii_lowercase().as_str() {
                "pass" => true,
                "fail" => false,
                _ => anyhow::bail!("--gate must be pass or fail"),
            };
            let used_doc_ids = used
                .as_deref()
                .map(parse_doc_ids)
                .transpose()?
                .unwrap_or_default();
            anyhow::ensure!(
                !passed || accepted_doc_id.is_some() || !used_doc_ids.is_empty(),
                "a passing pack needs <accepted_doc_id> or --used"
            );
            synapse_learn::feedback::record_context_outcome(
                &lstore,
                &query_id,
                accepted_doc_id,
                &used_doc_ids,
                passed,
                &shard_id,
            )?;
            let calibrated = synapse_learn::calibrate::update_calibration(&lstore)?;
            println!(
                "ok feedback gate={} accepted={} used={} shard={} calibrated_buckets={}",
                if passed { "pass" } else { "fail" },
                accepted_doc_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                used_doc_ids.len(),
                shard_id,
                calibrated
            );
        }
        Cmd::Backup {
            db,
            out,
            level,
            encrypt,
            passphrase,
        } => {
            let db_path = db.as_ref().unwrap_or(&cli.file);
            if encrypt {
                let pass = passphrase
                    .as_deref()
                    .context("--passphrase required with --encrypt")?;
                // Export plain first, then encrypt in-place
                let tmp = tempfile::NamedTempFile::new()?;
                snap::export(db_path, tmp.path(), level)?;
                snap::encrypt_pack(tmp.path(), &out, pass)?;
                println!("ok backup (encrypted) {}", out.display());
            } else {
                snap::export(db_path, &out, level)?;
                let meta = std::fs::metadata(&out)?;
                println!("ok backup {} ({} bytes)", out.display(), meta.len());
            }
        }
        Cmd::DbRestore {
            pack,
            db,
            passphrase,
        } => {
            let db_path = db.as_ref().unwrap_or(&cli.file);
            if let Some(p) = db_path.parent() {
                std::fs::create_dir_all(p).ok();
            }
            if let Some(pass) = passphrase.as_deref() {
                let tmp = tempfile::NamedTempFile::new()?;
                snap::decrypt_pack(&pack, tmp.path(), pass)?;
                snap::import(tmp.path(), db_path)?;
            } else {
                snap::import(&pack, db_path)?;
            }
            // Report doc count
            let store = Store::open(db_path)?;
            let count: i64 = store
                .conn
                .query_row("SELECT COUNT(*) FROM docs", [], |r| r.get(0))?;
            println!("ok restore {} docs → {}", count, db_path.display());
        }
        Cmd::DbVerify { db } => {
            let db_path = db.as_ref().unwrap_or(&cli.file);
            let store = Store::open(db_path)?;
            let mut stmt = store
                .conn
                .prepare("SELECT id, text, blake3 FROM docs ORDER BY id")?;
            let rows: Vec<VerifyRow> = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?;
            let total = rows.len();
            let mut bad = 0usize;
            for (id, text, stored_hash) in &rows {
                let computed = blake3::hash(text.as_bytes());
                if computed.as_bytes() != stored_hash.as_slice() {
                    eprintln!("CORRUPT id={id}");
                    bad += 1;
                }
            }
            if bad == 0 {
                println!("ok verify {} docs clean", total);
            } else {
                eprintln!("FAIL {bad}/{total} corrupt");
                std::process::exit(1);
            }
        }
        Cmd::DbRepair { db } => {
            let db_path = db.as_ref().unwrap_or(&cli.file);
            let store = Store::open(db_path)?;
            let report = safe_repair(&store, db_path)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Cmd::Import { src, format } => {
            let mut store = Store::open(&cli.file)?;
            let n = synx_io::import(&src, &mut store, format.as_deref())?;
            println!("ok import {} docs → {}", n, cli.file.display());
        }
        Cmd::Export { dst, format } => {
            let store = Store::open(&cli.file)?;
            let n = synx_io::export(&store, &dst, format.as_deref())?;
            println!("ok export {} docs → {}", n, dst.display());
        }
        Cmd::Convert { src, dst, from, to } => {
            let tmp = tempfile::NamedTempFile::new()?;
            let mut mid = Store::open(tmp.path())?;
            let n_in = synx_io::import(&src, &mut mid, from.as_deref())?;
            let n_out = synx_io::export(&mid, &dst, to.as_deref())?;
            println!(
                "ok convert {} docs: {} → {}",
                n_in.max(n_out),
                src.display(),
                dst.display()
            );
        }
        Cmd::Graph { action } => {
            let conn = rusqlite::Connection::open(&cli.file)?;
            synapse_graph::ensure_schema(&conn)?;
            match action {
                GraphCmd::Relate {
                    from,
                    to,
                    rel,
                    weight,
                } => {
                    synapse_graph::relate(&conn, from, to, &rel, weight, None)?;
                    println!(
                        "{{\"ok\":true,\"from\":{from},\"to\":{to},\"rel\":\"{rel}\",\"weight\":{weight}}}"
                    );
                }
                GraphCmd::Pagerank { n, damping, iters } => {
                    let top = synapse_graph::algorithms::top_pagerank(&conn, n, damping, iters)?;
                    println!("{}", serde_json::to_string_pretty(&top)?);
                }
                GraphCmd::Ppr {
                    seeds_json,
                    alpha,
                    iters,
                    limit,
                } => {
                    let seeds: std::collections::HashMap<String, f64> =
                        serde_json::from_str(&seeds_json).context("parse seeds JSON")?;
                    let seeds_i: std::collections::HashMap<i64, f64> = seeds
                        .into_iter()
                        .filter_map(|(k, v)| k.parse::<i64>().ok().map(|i| (i, v)))
                        .collect();
                    let ranked = synapse_core::ppr::personalized_pagerank(
                        &conn,
                        &seeds_i,
                        alpha,
                        iters,
                        synapse_core::ppr::DEFAULT_NEIGHBOR_CAP,
                        limit,
                    )?;
                    println!("{}", serde_json::to_string_pretty(&ranked)?);
                }
                GraphCmd::Communities { max_iters, top_n } => {
                    let comms = synapse_graph::algorithms::communities(&conn, max_iters)?;
                    let top: Vec<_> = comms.into_iter().take(top_n)
                        .map(|(id, members)| serde_json::json!({"community_id": id, "size": members.len(), "members": members}))
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&top)?);
                }
                GraphCmd::Neighbors {
                    node_id,
                    top_k,
                    rel,
                } => {
                    let n = synapse_graph::neighbors(&conn, node_id, rel.as_deref(), top_k)?;
                    println!("{}", serde_json::to_string_pretty(&n)?);
                }
                GraphCmd::Traverse {
                    start_id,
                    depth,
                    top_k_per_hop,
                    decay,
                } => {
                    let t = synapse_graph::traverse(
                        &conn,
                        start_id,
                        depth,
                        top_k_per_hop,
                        decay,
                        None,
                    )?;
                    println!("{}", serde_json::to_string_pretty(&t)?);
                }
                GraphCmd::Path {
                    from,
                    to,
                    max_depth,
                } => {
                    let p = synapse_graph::shortest_path(&conn, from, to, max_depth)?;
                    println!("{}", serde_json::to_string_pretty(&p)?);
                }
                GraphCmd::Count => {
                    let n = synapse_graph::edge_count(&conn)?;
                    println!("{{\"edges\":{n}}}");
                }
            }
        }
        Cmd::Ground {
            query,
            k,
            depth,
            alpha,
            iters,
        } => {
            // Pipeline: best available retrieval → seeds → PPR → traverse → JSON bundle.
            // Portable builds use lexical/timeline retrieval; semantic builds add hybrid retrieval.
            let store = Store::open(&cli.file)?;
            let (hits, _route) = search_best_effort(&store, &cli.file, &query, k)?;

            let mut seeds: std::collections::HashMap<i64, f64> = std::collections::HashMap::new();
            for h in &hits {
                seeds.insert(h.id, h.score);
            }

            let conn = rusqlite::Connection::open(&cli.file)?;
            synapse_graph::ensure_schema(&conn)?;

            let ppr_ranked = synapse_core::ppr::personalized_pagerank(
                &conn,
                &seeds,
                alpha,
                iters,
                synapse_core::ppr::DEFAULT_NEIGHBOR_CAP,
                30,
            )
            .unwrap_or_default();

            let mut expansions: Vec<serde_json::Value> = Vec::new();
            for (sid, _) in seeds.iter().take(8) {
                if let Ok(traverse_hits) = synapse_graph::traverse(&conn, *sid, depth, 6, 0.7, None)
                {
                    for (to_id, gscore, hop_depth, chain) in traverse_hits {
                        expansions.push(serde_json::json!({
                            "seed_id": sid, "to_id": to_id,
                            "score": gscore, "depth": hop_depth, "chain": chain
                        }));
                    }
                }
            }

            let bundle = serde_json::json!({
                "query": query,
                "version": 1,
                "hybrid_seeds": hits.iter().map(|h| serde_json::json!({"id": h.id, "score": h.score, "text": h.text.chars().take(120).collect::<String>()})).collect::<Vec<_>>(),
                "ppr_ranked": ppr_ranked,
                "graph_expansions": expansions,
                "params": {"k": k, "depth": depth, "alpha": alpha, "iters": iters},
            });
            println!("{}", serde_json::to_string_pretty(&bundle)?);
        }
        Cmd::Corpus { action } => {
            let store = Store::open(&cli.file)?;
            corpus_migrate(&store.conn)?;
            match action {
                CorpusCmd::AddText {
                    kind,
                    source_uri,
                    external_id,
                    title,
                    text,
                    published_ts,
                    embed,
                } => {
                    let body = match text {
                        Some(t) => t,
                        None => {
                            let mut s = String::new();
                            std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
                            s.trim().to_string()
                        }
                    };
                    anyhow::ensure!(!body.trim().is_empty(), "empty text");
                    let doc = NewCorpusDocument {
                        source_kind: parse_corpus_kind(&kind)?,
                        source_uri: &source_uri,
                        external_id: &external_id,
                        title: &title,
                        text: &body,
                        published_ts,
                    };
                    let doc_id = put_corpus_document(&store.conn, &doc)?;
                    let mut embedded = 0usize;
                    if embed {
                        embedded = embed_corpus_documents(&store.conn, &cli.file, &[doc_id])?;
                    }
                    println!("ok corpus_doc id={} embedded_chunks={}", doc_id, embedded);
                }
                CorpusCmd::AddRss {
                    source_uri,
                    path,
                    embed,
                } => {
                    let xml = match path {
                        Some(path) => std::fs::read_to_string(&path)
                            .with_context(|| format!("read {}", path.display()))?,
                        None => {
                            let mut s = String::new();
                            std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
                            s
                        }
                    };
                    anyhow::ensure!(!xml.trim().is_empty(), "empty RSS XML");
                    let doc_ids = ingest_rss_xml(&store.conn, &source_uri, &xml)?;
                    let embedded = if embed {
                        embed_corpus_documents(&store.conn, &cli.file, &doc_ids)?
                    } else {
                        0
                    };
                    println!(
                        "ok rss_docs count={} embedded_chunks={}",
                        doc_ids.len(),
                        embedded
                    );
                }
                CorpusCmd::AddYoutube {
                    video_id,
                    title,
                    source_uri,
                    path,
                    embed,
                } => {
                    let transcript = match path {
                        Some(path) => std::fs::read_to_string(&path)
                            .with_context(|| format!("read {}", path.display()))?,
                        None => {
                            let mut s = String::new();
                            std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
                            s
                        }
                    };
                    anyhow::ensure!(!transcript.trim().is_empty(), "empty YouTube transcript");
                    let video_uri = source_uri
                        .unwrap_or_else(|| format!("https://www.youtube.com/watch?v={video_id}"));
                    let doc_id = ingest_youtube_transcript(
                        &store.conn,
                        &video_uri,
                        &video_id,
                        &title,
                        &transcript,
                    )?;
                    let embedded = if embed {
                        embed_corpus_documents(&store.conn, &cli.file, &[doc_id])?
                    } else {
                        0
                    };
                    println!("ok youtube_doc id={} embedded_chunks={}", doc_id, embedded);
                }
                CorpusCmd::FetchYoutube {
                    url,
                    video_id,
                    title,
                    lang,
                    yt_dlp,
                    embed,
                } => {
                    let video_id = video_id
                        .or_else(|| youtube_video_id(&url))
                        .ok_or_else(|| anyhow::anyhow!("could not infer YouTube video id"))?;
                    let transcript = fetch_youtube_transcript_with_ytdlp(&yt_dlp, &url, &lang)?;
                    let title = title.unwrap_or_else(|| format!("YouTube {video_id}"));
                    let doc_id = ingest_youtube_transcript(
                        &store.conn,
                        &url,
                        &video_id,
                        &title,
                        &transcript,
                    )?;
                    let embedded = if embed {
                        embed_corpus_documents(&store.conn, &cli.file, &[doc_id])?
                    } else {
                        0
                    };
                    println!("ok youtube_doc id={} embedded_chunks={}", doc_id, embedded);
                }
                CorpusCmd::AddWeb {
                    source_uri,
                    title,
                    path,
                    embed,
                } => {
                    let html = match path {
                        Some(path) => std::fs::read_to_string(&path)
                            .with_context(|| format!("read {}", path.display()))?,
                        None => {
                            let mut s = String::new();
                            std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
                            s
                        }
                    };
                    anyhow::ensure!(!html.trim().is_empty(), "empty web HTML");
                    let doc_id =
                        ingest_web_html(&store.conn, &source_uri, title.as_deref(), &html)?;
                    let embedded = if embed {
                        embed_corpus_documents(&store.conn, &cli.file, &[doc_id])?
                    } else {
                        0
                    };
                    println!("ok web_doc id={} embedded_chunks={}", doc_id, embedded);
                }
                CorpusCmd::AddPdf {
                    source_uri,
                    title,
                    path,
                    embed,
                } => {
                    let bytes = match path {
                        Some(path) => std::fs::read(&path)
                            .with_context(|| format!("read {}", path.display()))?,
                        None => {
                            let mut bytes = Vec::new();
                            std::io::Read::read_to_end(&mut std::io::stdin(), &mut bytes)?;
                            bytes
                        }
                    };
                    anyhow::ensure!(!bytes.is_empty(), "empty PDF");
                    let doc_id = ingest_pdf_bytes(&store.conn, &source_uri, &title, &bytes)?;
                    let embedded = if embed {
                        embed_corpus_documents(&store.conn, &cli.file, &[doc_id])?
                    } else {
                        0
                    };
                    println!("ok pdf_doc id={} embedded_chunks={}", doc_id, embedded);
                }
                CorpusCmd::FetchUrl {
                    url,
                    title,
                    content_type,
                    embed,
                } => {
                    let (header_content_type, bytes) = fetch_url_bytes(&url)?;
                    let routed_content_type = content_type.or(header_content_type);
                    let doc_ids = ingest_fetched_document(
                        &store.conn,
                        &url,
                        routed_content_type.as_deref(),
                        title.as_deref(),
                        &bytes,
                    )?;
                    let embedded = if embed {
                        embed_corpus_documents(&store.conn, &cli.file, &doc_ids)?
                    } else {
                        0
                    };
                    println!(
                        "ok fetched_docs count={} embedded_chunks={}",
                        doc_ids.len(),
                        embedded
                    );
                }
                CorpusCmd::WatchUrl {
                    url,
                    kind,
                    title,
                    every_secs,
                } => {
                    let id = upsert_corpus_sync_source(
                        &store.conn,
                        parse_corpus_kind(&kind)?,
                        &url,
                        title.as_deref(),
                        every_secs,
                    )?;
                    println!("ok watched_source id={} every_secs={}", id, every_secs);
                }
                CorpusCmd::SyncDue { limit, embed } => {
                    let now = unix_now_secs();
                    let due = due_corpus_sync_sources(&store.conn, now, limit)?;
                    let mut synced = 0usize;
                    let mut docs = 0usize;
                    let mut embedded = 0usize;
                    for source in due {
                        let (content_type, bytes) = fetch_url_bytes(&source.uri)?;
                        let doc_ids = ingest_fetched_document(
                            &store.conn,
                            &source.uri,
                            content_type.as_deref(),
                            source.title.as_deref(),
                            &bytes,
                        )?;
                        if embed {
                            embedded += embed_corpus_documents(&store.conn, &cli.file, &doc_ids)?;
                        }
                        mark_corpus_source_synced(&store.conn, source.id, now)?;
                        docs += doc_ids.len();
                        synced += 1;
                    }
                    println!(
                        "ok sync_due sources={} docs={} embedded_chunks={}",
                        synced, docs, embedded
                    );
                }
                CorpusCmd::ImportSynapse { limit, embed } => {
                    let doc_ids = import_synapse_docs_to_corpus(&store.conn, limit)?;
                    let embedded = if embed {
                        embed_corpus_documents(&store.conn, &cli.file, &doc_ids)?
                    } else {
                        0
                    };
                    println!(
                        "ok imported_synapse_docs count={} embedded_chunks={}",
                        doc_ids.len(),
                        embedded
                    );
                }
                CorpusCmd::Search {
                    query,
                    limit,
                    embed,
                    json,
                } => {
                    let q = if embed {
                        Some(semantic_embedding(&cli.file, &query)?)
                    } else {
                        None
                    };
                    let hits = search_corpus(&store.conn, &query, q.as_deref(), limit, None)?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&hits_to_json(&hits))?);
                    } else {
                        print_corpus_hits(&hits);
                    }
                }
                CorpusCmd::Promote {
                    chunk_id,
                    kind,
                    rationale,
                } => {
                    let id = queue_promotion(
                        &store.conn,
                        chunk_id,
                        parse_promotion_kind(&kind)?,
                        &rationale,
                    )?;
                    println!("ok promotion queued id={id} chunk_id={chunk_id}");
                }
                CorpusCmd::Verify {
                    promotion_id,
                    verifier,
                } => {
                    verify_promotion(&store.conn, promotion_id, &verifier)?;
                    println!("ok promotion verified id={promotion_id}");
                }
                CorpusCmd::Ready => {
                    let ids = ready_promotions(&store.conn)?;
                    println!("{}", serde_json::to_string_pretty(&ids)?);
                }
                CorpusCmd::GoldCandidates { limit } => {
                    let candidates = gold_candidates_from_corpus(&store.conn, limit)?;
                    println!("{}", serde_json::to_string_pretty(&candidates)?);
                }
                CorpusCmd::BaselineRankings {
                    gold_json,
                    limit,
                    embed,
                } => {
                    let gold = read_gold_questions(&gold_json)?;
                    let rankings =
                        rank_gold_questions_cli(&store.conn, &cli.file, &gold, limit, embed)?;
                    println!("{}", serde_json::to_string_pretty(&rankings)?);
                }
                CorpusCmd::BootstrapEval {
                    limit,
                    min_gold,
                    rank_limit,
                    import_synapse,
                    candidates_json,
                    gold_json,
                    baseline_rankings_json,
                } => {
                    let imported = if import_synapse {
                        import_synapse_docs_to_corpus(&store.conn, limit)?.len()
                    } else {
                        0
                    };
                    let boot =
                        bootstrap_eval_from_corpus(&store.conn, limit, min_gold, rank_limit)?;
                    if let Some(path) = candidates_json.as_deref() {
                        write_json_file(path, &boot.candidates)?;
                    }
                    if let Some(path) = gold_json.as_deref() {
                        write_json_file(path, &boot.gold)?;
                    }
                    if let Some(path) = baseline_rankings_json.as_deref() {
                        write_json_file(path, &boot.baseline_rankings)?;
                    }
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "imported_synapse_docs": imported,
                            "gold_count": boot.gold_count,
                            "min_gold": boot.min_gold,
                            "baseline": boot.baseline,
                            "candidates_json": candidates_json,
                            "gold_json": gold_json,
                            "baseline_rankings_json": baseline_rankings_json,
                        }))?
                    );
                }
                CorpusCmd::Eval {
                    gold_json,
                    limit,
                    embed,
                } => {
                    let gold = read_gold_questions(&gold_json)?;
                    let rankings =
                        rank_gold_questions_cli(&store.conn, &cli.file, &gold, limit, embed)?;
                    let report = evaluate_rankings(&gold, &rankings)?;
                    println!("{}", serde_json::to_string_pretty(&report)?);
                }
                CorpusCmd::EvalGate {
                    gold_json,
                    baseline_rankings_json,
                    limit,
                    min_gold,
                    embed,
                } => {
                    let gold = read_gold_questions(&gold_json)?;
                    let raw_baseline = std::fs::read_to_string(&baseline_rankings_json)
                        .with_context(|| format!("read {}", baseline_rankings_json.display()))?;
                    let baseline_rankings: Vec<Vec<i64>> =
                        serde_json::from_str(&raw_baseline).context("parse baseline rankings")?;
                    let candidate_rankings =
                        rank_gold_questions_cli(&store.conn, &cli.file, &gold, limit, embed)?;
                    let report = evaluate_rankings_gate(
                        &gold,
                        &baseline_rankings,
                        &candidate_rankings,
                        min_gold,
                    )?;
                    println!("{}", serde_json::to_string_pretty(&report)?);
                    anyhow::ensure!(report.passed, "corpus eval gate failed");
                }
            }
        }
    }
    Ok(())
}

fn parse_fresh_input(raw: &str) -> FreshInput {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return (String::new(), None, None);
    }

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed)
        && let Some(obj) = value.as_object()
    {
        let prompt = obj
            .get("prompt")
            .or_else(|| obj.get("input"))
            .or_else(|| obj.get("message"))
            .map(|v| {
                v.as_str()
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| v.to_string())
            })
            .unwrap_or_else(|| trimmed.to_string());
        let cwd = obj.get("cwd").and_then(|v| v.as_str()).map(PathBuf::from);
        let project = obj
            .get("project")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned);
        return (prompt, cwd, project);
    }

    (trimmed.to_string(), None, None)
}

fn unix_now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn print_hits(hits: &[synapse_core::Hit]) {
    for h in hits {
        let title = h.title.as_deref().unwrap_or("");
        let uri = h.uri.as_deref().unwrap_or("");
        let snippet = h.text.chars().take(120).collect::<String>();
        if title.is_empty() && uri.is_empty() {
            println!("{}\t{:.4}\t{}", h.id, h.score, snippet);
        } else {
            println!("{}\t{:.4}\t{}\t{}\t{}", h.id, h.score, title, uri, snippet);
        }
    }
}

fn parse_corpus_kind(raw: &str) -> Result<CorpusSourceKind> {
    match raw {
        "rss" => Ok(CorpusSourceKind::Rss),
        "youtube" | "yt" => Ok(CorpusSourceKind::Youtube),
        "pdf" => Ok(CorpusSourceKind::Pdf),
        "web" | "url" | "article" => Ok(CorpusSourceKind::Web),
        "text" | "manual" | "note" => Ok(CorpusSourceKind::Text),
        other => Err(anyhow::anyhow!(
            "invalid corpus kind {other:?}; expected rss|youtube|pdf|web|text"
        )),
    }
}

fn parse_promotion_kind(raw: &str) -> Result<PromotionKind> {
    match raw {
        "fact" => Ok(PromotionKind::Fact),
        "decision" => Ok(PromotionKind::Decision),
        other => Err(anyhow::anyhow!(
            "invalid promotion kind {other:?}; expected fact|decision"
        )),
    }
}

fn fetch_youtube_transcript_with_ytdlp(
    yt_dlp: &std::path::Path,
    url: &str,
    lang: &str,
) -> Result<String> {
    let dir = tempfile::tempdir().context("create yt-dlp tempdir")?;
    let template = dir.path().join("%(id)s.%(ext)s");
    let output = std::process::Command::new(yt_dlp)
        .arg("--skip-download")
        .arg("--write-subs")
        .arg("--write-auto-subs")
        .arg("--sub-langs")
        .arg(lang)
        .arg("--sub-format")
        .arg("vtt/srt")
        .arg("--no-playlist")
        .arg("-o")
        .arg(&template)
        .arg(url)
        .output()
        .with_context(|| format!("run {}", yt_dlp.display()))?;
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "yt-dlp failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let mut transcript_paths = Vec::new();
    for entry in std::fs::read_dir(dir.path()).context("read yt-dlp tempdir")? {
        let path = entry?.path();
        let ext = path
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or_default();
        if matches!(ext, "vtt" | "srt") {
            transcript_paths.push(path);
        }
    }
    transcript_paths.sort();
    let path = transcript_paths
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("yt-dlp did not produce a .vtt or .srt transcript"))?;
    let transcript =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    anyhow::ensure!(!transcript.trim().is_empty(), "empty YouTube transcript");
    Ok(transcript)
}

#[cfg(any(feature = "static-ort", feature = "cross-linux"))]
fn embed_corpus_documents(
    conn: &rusqlite::Connection,
    brain_file: &std::path::Path,
    doc_ids: &[i64],
) -> Result<usize> {
    let e = Embedder::new_with_cache::<std::path::PathBuf>(
        brain_file.parent().map(|p| p.join(".emb-cache")),
    )
    .context("embedder init")?;
    let mut embedded = 0usize;
    for doc_id in doc_ids {
        let mut stmt = conn.prepare(
            "SELECT id, text FROM synapse_corpus_chunks WHERE document_id=?1 ORDER BY ordinal",
        )?;
        let chunks: Vec<(i64, String)> = stmt
            .query_map([doc_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (chunk_id, chunk_text) in chunks {
            let emb = e.embed_one(&chunk_text)?;
            set_corpus_chunk_embedding(conn, chunk_id, &emb)?;
            embedded += 1;
        }
    }
    Ok(embedded)
}

#[cfg(not(any(feature = "static-ort", feature = "cross-linux")))]
fn embed_corpus_documents(
    _conn: &rusqlite::Connection,
    _brain_file: &std::path::Path,
    _doc_ids: &[i64],
) -> Result<usize> {
    Err(anyhow::anyhow!(
        "corpus embedding requires a semantic build; rerun without --embed on the portable build"
    ))
}

fn read_gold_questions(path: &std::path::Path) -> Result<Vec<GoldQuestion>> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&raw).context("parse gold JSON")
}

fn write_json_file<T: serde::Serialize>(path: &std::path::Path, value: &T) -> Result<()> {
    let raw = serde_json::to_vec_pretty(value)?;
    std::fs::write(path, raw).with_context(|| format!("write {}", path.display()))
}

#[cfg(any(feature = "static-ort", feature = "cross-linux"))]
fn rank_gold_questions_cli(
    conn: &rusqlite::Connection,
    brain_file: &std::path::Path,
    gold: &[GoldQuestion],
    limit: usize,
    embed: bool,
) -> Result<Vec<Vec<i64>>> {
    if !embed {
        return rank_gold_questions(conn, gold, limit).map_err(Into::into);
    }
    let e = Embedder::new_with_cache::<std::path::PathBuf>(
        brain_file.parent().map(|p| p.join(".emb-cache")),
    )
    .context("embedder init")?;
    let mut rankings = Vec::with_capacity(gold.len());
    for q in gold {
        let emb = e.embed_one(&q.query)?;
        let hits = search_corpus(conn, &q.query, Some(&emb), limit, None)?;
        rankings.push(hits.into_iter().map(|h| h.chunk_id).collect());
    }
    Ok(rankings)
}

#[cfg(not(any(feature = "static-ort", feature = "cross-linux")))]
fn rank_gold_questions_cli(
    conn: &rusqlite::Connection,
    _brain_file: &std::path::Path,
    gold: &[GoldQuestion],
    limit: usize,
    embed: bool,
) -> Result<Vec<Vec<i64>>> {
    if embed {
        return Err(anyhow::anyhow!(
            "corpus evaluation with embeddings requires a semantic build; rerun without --embed on the portable build"
        ));
    }
    rank_gold_questions(conn, gold, limit).map_err(Into::into)
}

fn print_corpus_hits(hits: &[synapse_core::corpus::CorpusHit]) {
    for h in hits {
        let snippet = h.text.chars().take(160).collect::<String>();
        println!(
            "{}\t{}\t{:.6}\t{}\t{}",
            h.chunk_id, h.document_id, h.score, h.title, snippet
        );
    }
}

fn hits_to_json(hits: &[synapse_core::corpus::CorpusHit]) -> Vec<serde_json::Value> {
    hits.iter()
        .map(|h| {
            serde_json::json!({
                "chunk_id": h.chunk_id,
                "document_id": h.document_id,
                "title": h.title,
                "score": h.score,
                "text": h.text,
            })
        })
        .collect()
}

#[derive(serde::Serialize)]
struct PrimeReport {
    project: String,
    root: String,
    mode: String,
    git: PrimeGit,
    source_docs: Vec<PrimeSourceDoc>,
    commands: Vec<String>,
    memory_route: String,
    memories: Vec<PrimeMemory>,
    fresh_command: String,
    doctor_command: String,
    context_command: String,
    feedback_hint: String,
}

#[derive(serde::Serialize)]
struct PrimeGit {
    branch: Option<String>,
    head: Option<String>,
    dirty_files: usize,
    root: Option<String>,
}

#[derive(serde::Serialize)]
struct PrimeSourceDoc {
    path: String,
    heading: Option<String>,
}

#[derive(serde::Serialize)]
struct PrimeMemory {
    id: i64,
    score: f64,
    title: Option<String>,
    source: Option<String>,
    snippet: String,
}

fn build_prime_report(
    root: &std::path::Path,
    store: Option<&Store>,
    brain_file: &std::path::Path,
    mode: &str,
    limit: usize,
) -> Result<PrimeReport> {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let project = root
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("project")
        .to_string();
    let git = prime_git(&root);
    let source_docs = prime_source_docs(&root);
    let commands = prime_commands(&root);
    let query = format!(
        "{} {} decision fact bugfix benchmark preference research verified context",
        project, mode
    );
    let (memories, memory_route) = if let Some(store) = store {
        let (hits, route) = search_best_effort(store, brain_file, &query, limit)
            .unwrap_or_else(|_| (Vec::new(), "unavailable".to_string()));
        let ranked = rank_context_hits(store, None, hits).unwrap_or_default();
        (
            ranked
                .into_iter()
                .take(limit)
                .map(|h| PrimeMemory {
                    id: h.id,
                    score: h.score,
                    title: h.title,
                    source: h.uri,
                    snippet: compact(&h.text, 260),
                })
                .collect(),
            route,
        )
    } else {
        (Vec::new(), "unavailable".to_string())
    };

    let prompt = format!("latest package API version notes for {}", project);
    Ok(PrimeReport {
        project: project.clone(),
        root: root.display().to_string(),
        mode: mode.to_string(),
        git,
        source_docs,
        commands,
        memory_route,
        memories,
        fresh_command: format!(
            "synx -f {} fresh-context --cwd {} --prompt {:?}",
            shell_path(brain_file),
            shell_path(&root),
            prompt
        ),
        doctor_command: format!("synx -f {} doctor --json", shell_path(brain_file)),
        context_command: format!(
            "synx -f {} context {:?} --mode {}",
            shell_path(brain_file),
            format!("{} current task", project),
            mode
        ),
        feedback_hint: "After using a memory: synx feedback context:<context_id> <doc_id>"
            .to_string(),
    })
}

fn print_prime_report(report: &PrimeReport) {
    println!("# Synapse Agent Memory Prime Brief");
    println!();
    println!("project: {}", report.project);
    println!("root: {}", report.root);
    println!("mode: {}", report.mode);
    println!();
    println!("## Git");
    println!(
        "branch={} head={} dirty_files={} git_root={}",
        report.git.branch.as_deref().unwrap_or("unknown"),
        report.git.head.as_deref().unwrap_or("unknown"),
        report.git.dirty_files,
        report.git.root.as_deref().unwrap_or("unknown")
    );
    println!();
    println!("## Source docs");
    if report.source_docs.is_empty() {
        println!("- none detected");
    } else {
        for doc in &report.source_docs {
            match doc.heading.as_deref() {
                Some(heading) => println!("- {} — {}", doc.path, heading),
                None => println!("- {}", doc.path),
            }
        }
    }
    println!();
    println!("## Commands");
    for cmd in &report.commands {
        println!("- {}", cmd);
    }
    println!("- {}", report.doctor_command);
    println!("- {}", report.fresh_command);
    println!();
    println!("## Recent/relevant memory");
    println!("route: {}", report.memory_route);
    if report.memories.is_empty() {
        println!("- none yet; add durable context with `synx remember --kind decision ...`");
    } else {
        for mem in &report.memories {
            println!(
                "- [{}] {:.4} {} :: {}",
                mem.id,
                mem.score,
                mem.title.as_deref().unwrap_or("untitled"),
                mem.snippet
            );
        }
    }
    println!();
    println!("## Next agent steps");
    println!("- Start with: {}", report.context_command);
    println!("- For version-sensitive work, run the fresh-context command above.");
    println!("- {}", report.feedback_hint);
}

fn prime_git(root: &std::path::Path) -> PrimeGit {
    PrimeGit {
        branch: git_output(root, &["branch", "--show-current"]),
        head: git_output(root, &["log", "-1", "--format=%h %s"]),
        dirty_files: git_output(root, &["status", "--short"])
            .map(|s| s.lines().filter(|line| !line.trim().is_empty()).count())
            .unwrap_or(0),
        root: git_output(root, &["rev-parse", "--show-toplevel"]),
    }
}

fn git_output(root: &std::path::Path, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn prime_source_docs(root: &std::path::Path) -> Vec<PrimeSourceDoc> {
    let candidates = [
        "AGENTS.md",
        "CLAUDE.md",
        "README.md",
        "SPEC.md",
        "docs/SPEC.md",
        "docs/CONTEXT_OS_PLAN_2026-05-18.md",
        "docs/SPEC-VS-REALITY-2026-05-04.md",
        "docs/ROADMAP.md",
        "docs/adr",
        "openspec",
        "justfile",
        "mise.toml",
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        ".env.example",
    ];
    let mut docs = Vec::new();
    for rel in candidates {
        let path = root.join(rel);
        if path.is_dir() {
            docs.push(PrimeSourceDoc {
                path: rel.to_string(),
                heading: Some("directory present".to_string()),
            });
        } else if path.is_file() {
            docs.push(PrimeSourceDoc {
                path: rel.to_string(),
                heading: first_heading(&path),
            });
        }
    }
    docs
}

fn first_heading(path: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .map(str::trim)
        .find(|line| line.starts_with("# ") || line.starts_with("## "))
        .map(|line| line.trim_start_matches('#').trim().to_string())
}

fn prime_commands(root: &std::path::Path) -> Vec<String> {
    let mut commands = Vec::new();
    if root.join("justfile").is_file() {
        commands.extend([
            "just doctor".to_string(),
            "just check".to_string(),
            "just test".to_string(),
        ]);
    }
    if root.join("Cargo.toml").is_file() {
        commands.extend([
            "cargo fmt --check".to_string(),
            "cargo check --workspace".to_string(),
            "cargo test --workspace".to_string(),
        ]);
    }
    if root.join("package.json").is_file() {
        commands.extend(["bun install".to_string(), "bun test".to_string()]);
    }
    if root.join("pyproject.toml").is_file() {
        commands.push("uv run pytest".to_string());
    }
    if commands.is_empty() {
        commands.push("inspect project docs for check/test commands".to_string());
    }
    commands
}

fn shell_path(path: &std::path::Path) -> String {
    let s = path.display().to_string();
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._:-".contains(c))
    {
        s
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

fn build_put_meta(
    source: Option<String>,
    updated: Option<String>,
    kind: Option<String>,
    status: Option<String>,
    meta_json: Option<String>,
) -> Result<Option<serde_json::Value>> {
    let mut map = match meta_json {
        Some(raw) => {
            let value: serde_json::Value = serde_json::from_str(&raw)
                .with_context(|| format!("invalid --meta JSON: {raw}"))?;
            value
                .as_object()
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("--meta must be a JSON object"))?
        }
        None => serde_json::Map::new(),
    };

    if let Some(value) = source {
        map.insert("source".to_string(), serde_json::Value::String(value));
    }
    if let Some(value) = updated {
        map.insert("updated".to_string(), serde_json::Value::String(value));
    }
    if let Some(value) = kind {
        map.insert("kind".to_string(), serde_json::Value::String(value));
    }
    if let Some(value) = status {
        map.insert("status".to_string(), serde_json::Value::String(value));
    }

    Ok((!map.is_empty()).then_some(serde_json::Value::Object(map)))
}

fn search_best_effort(
    store: &Store,
    file: &std::path::Path,
    query: &str,
    limit: usize,
) -> Result<SearchBestEffortResult> {
    let lex = store
        .search(query, SearchMode::Lex, None, limit)
        .unwrap_or_default();
    if !lex.is_empty() {
        return Ok((lex, "lexical".to_string()));
    }

    let hybrid = semantic_embedding(file, query)
        .ok()
        .and_then(|q| {
            store
                .search(query, SearchMode::Hybrid, Some(&q), limit)
                .ok()
        })
        .unwrap_or_default();
    if !hybrid.is_empty() {
        return Ok((hybrid, "hybrid".to_string()));
    }

    let docs = store.timeline(limit, 0)?;
    Ok((
        docs.into_iter().map(doc_to_hit).collect(),
        "timeline".to_string(),
    ))
}

fn doc_to_hit(doc: synapse_core::Doc) -> synapse_core::Hit {
    synapse_core::Hit {
        id: doc.id,
        uri: doc.uri,
        title: doc.title,
        text: doc.text,
        score: 0.0,
        meta: doc.meta,
        ts: Some(doc.ts),
    }
}

#[derive(Debug, Default, Clone, serde::Serialize)]
struct ContextDiagnostics {
    candidates_seen: usize,
    noise_filtered: usize,
    superseded_filtered: usize,
    temporal_filtered: usize,
    temporal_lo: Option<String>,
    temporal_hi: Option<String>,
    retrieval_query: Option<String>,
}

impl ContextDiagnostics {
    fn with_range(range: Option<DateRange>) -> Self {
        Self {
            temporal_lo: range.and_then(|value| format_timestamp(value.lo)),
            temporal_hi: range.and_then(|value| format_timestamp(value.hi)),
            ..Self::default()
        }
    }

    fn merge(&mut self, other: Self) {
        self.candidates_seen += other.candidates_seen;
        self.noise_filtered += other.noise_filtered;
        self.superseded_filtered += other.superseded_filtered;
        self.temporal_filtered += other.temporal_filtered;
        self.temporal_lo = self.temporal_lo.take().or(other.temporal_lo);
        self.temporal_hi = self.temporal_hi.take().or(other.temporal_hi);
        self.retrieval_query = self.retrieval_query.take().or(other.retrieval_query);
    }
}

struct ContextOutput<'a> {
    context_id: &'a str,
    query: &'a str,
    mode: &'a str,
    budget: usize,
    used_chars: usize,
    route: &'a str,
    hits: &'a [synapse_core::Hit],
    diagnostics: &'a ContextDiagnostics,
}

fn rank_context_hits(
    store: &Store,
    learn: Option<&LearnStore>,
    hits: Vec<synapse_core::Hit>,
) -> Result<Vec<synapse_core::Hit>> {
    Ok(rank_context_hits_in_range(store, learn, hits, None)?.0)
}

fn rank_context_hits_in_range(
    store: &Store,
    learn: Option<&LearnStore>,
    hits: Vec<synapse_core::Hit>,
    temporal_range: Option<DateRange>,
) -> Result<(Vec<synapse_core::Hit>, ContextDiagnostics)> {
    let mut diagnostics = ContextDiagnostics::with_range(temporal_range);
    diagnostics.candidates_seen = hits.len();
    let mut filtered = Vec::with_capacity(hits.len());
    for hit in hits {
        if is_context_noise(&hit) {
            diagnostics.noise_filtered += 1;
            continue;
        }
        if doc_memory_state(&store.conn, hit.id)?.is_some_and(|state| state.superseded) {
            diagnostics.superseded_filtered += 1;
            continue;
        }
        if let Some(range) = temporal_range {
            let occurred = context_hit_event_ts(&hit);
            if !occurred.is_some_and(|ts| (range.lo..=range.hi).contains(&ts)) {
                diagnostics.temporal_filtered += 1;
                continue;
            }
        }
        filtered.push(hit);
    }

    let min_score = filtered
        .iter()
        .map(|hit| hit.score)
        .fold(f64::INFINITY, f64::min);
    let max_score = filtered
        .iter()
        .map(|hit| hit.score)
        .fold(f64::NEG_INFINITY, f64::max);
    let spread = max_score - min_score;
    let mut ranked = Vec::with_capacity(filtered.len());
    for mut hit in filtered {
        let mut score = if spread.is_finite() && spread > f64::EPSILON {
            (hit.score - min_score) / spread
        } else {
            0.5
        };
        if let Some(learn) = learn {
            score = synapse_learn::calibrate::calibrate(learn, score).unwrap_or(score);
        }
        let kind = context_hit_kind(&hit);
        score += memory_kind_prior(&kind);
        score += priority_bonus(context_hit_priority(&hit));
        if let Some(state) = doc_memory_state(&store.conn, hit.id)? {
            score += (state.confidence.clamp(0.0, 1.0) - 0.5) * 0.08;
        }
        if let Some(learn) = learn {
            score += learn.memory_type_bonus(&kind).unwrap_or(0.0);
        }
        hit.score = score;
        ranked.push(hit);
    }
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok((ranked, diagnostics))
}

fn memory_kind_prior(kind: &str) -> f64 {
    match kind {
        "decision" => 0.090,
        "fact" => 0.075,
        "bugfix" => 0.065,
        "benchmark" => 0.060,
        "preference" => 0.055,
        "adr" => 0.052,
        "command" => 0.045,
        "research" => 0.035,
        "session" => 0.010,
        _ => 0.0,
    }
}

fn priority_bonus(priority: &str) -> f64 {
    match priority {
        "critical" => 0.120,
        "high" => 0.070,
        "low" => -0.050,
        _ => 0.0,
    }
}

fn context_hit_kind(hit: &synapse_core::Hit) -> String {
    hit.meta
        .as_ref()
        .and_then(|meta| meta.get("kind"))
        .and_then(|value| value.as_str())
        .unwrap_or("note")
        .to_ascii_lowercase()
}

fn context_hit_priority(hit: &synapse_core::Hit) -> &str {
    hit.meta
        .as_ref()
        .and_then(|meta| meta.get("priority"))
        .and_then(|value| value.as_str())
        .unwrap_or("normal")
}

fn context_hit_event_ts(hit: &synapse_core::Hit) -> Option<i64> {
    let meta_ts = hit
        .meta
        .as_ref()
        .and_then(|meta| meta.get("occurred_ts"))
        .and_then(|value| value.as_i64())
        .or_else(|| {
            hit.meta
                .as_ref()
                .and_then(|meta| meta.get("occurred_at"))
                .and_then(|value| value.as_str())
                .and_then(parse_timestamp)
        });
    meta_ts.or_else(|| hit.ts.map(|millis| millis / 1000))
}

fn context_hit_captured_at(hit: &synapse_core::Hit) -> Option<String> {
    hit.ts.and_then(|millis| format_timestamp(millis / 1000))
}

fn context_hit_occurred_at(hit: &synapse_core::Hit) -> Option<String> {
    hit.meta
        .as_ref()
        .and_then(|meta| meta.get("occurred_at"))
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
}

fn is_context_noise(hit: &synapse_core::Hit) -> bool {
    let status = hit
        .meta
        .as_ref()
        .and_then(|meta| meta.get("status"))
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let kind = context_hit_kind(hit);
    if matches!(
        status.as_str(),
        "stale" | "archived" | "noise" | "generated"
    ) || matches!(kind.as_str(), "status" | "telepathy" | "notification")
    {
        return true;
    }
    let text = hit.text.to_ascii_lowercase();
    text.trim_start().starts_with("[telepathy]")
        || text.contains("<task-notification>")
        || text.contains("tool-use-id")
        || (text.contains("\"models_loaded\"")
            && text.contains("\"desktop_procs\"")
            && text.contains("\"cli_sessions\""))
}

fn temporal_retrieval_query(query: &str) -> String {
    let raw: Vec<&str> = query.split_whitespace().collect();
    let tokens: Vec<String> = raw
        .iter()
        .map(|token| {
            token
                .trim_matches(|c: char| !(c.is_alphanumeric() || c == '-' || c == '/'))
                .to_ascii_lowercase()
        })
        .collect();
    let is_year = |value: &str| value.len() == 4 && value.chars().all(|c| c.is_ascii_digit());
    let is_quarter = |value: &str| matches!(value, "q1" | "q2" | "q3" | "q4");
    let is_month = |value: &str| {
        matches!(
            value,
            "january"
                | "jan"
                | "januar"
                | "february"
                | "feb"
                | "februar"
                | "march"
                | "mar"
                | "märz"
                | "maerz"
                | "april"
                | "apr"
                | "may"
                | "mai"
                | "june"
                | "jun"
                | "juni"
                | "july"
                | "jul"
                | "juli"
                | "august"
                | "aug"
                | "september"
                | "sep"
                | "sept"
                | "october"
                | "oct"
                | "oktober"
                | "okt"
                | "november"
                | "nov"
                | "december"
                | "dec"
                | "dezember"
                | "dez"
        )
    };
    let is_unit = |value: &str| {
        matches!(
            value,
            "day"
                | "days"
                | "week"
                | "weeks"
                | "month"
                | "months"
                | "year"
                | "years"
                | "tag"
                | "tage"
                | "tagen"
                | "woche"
                | "wochen"
                | "monat"
                | "monate"
                | "monaten"
                | "jahr"
                | "jahre"
                | "jahren"
        )
    };
    let is_cue = |value: &str| {
        matches!(
            value,
            "yesterday"
                | "today"
                | "tomorrow"
                | "last"
                | "this"
                | "next"
                | "ago"
                | "in"
                | "gestern"
                | "heute"
                | "morgen"
                | "letzte"
                | "letzten"
                | "letzter"
                | "letztes"
                | "diese"
                | "dieser"
                | "diesen"
                | "dieses"
                | "nächste"
                | "nächsten"
                | "nächstes"
                | "naechste"
                | "naechsten"
                | "naechstes"
                | "vor"
                | "im"
        )
    };

    let mut keep = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let previous = index.checked_sub(1).and_then(|value| tokens.get(value));
        let next = tokens.get(index + 1);
        let explicit_date = parse_timestamp(token).is_some();
        let numeric_relative = token.chars().all(|c| c.is_ascii_digit())
            && (previous.is_some_and(|value| matches!(value.as_str(), "in" | "vor"))
                || next.is_some_and(|value| is_unit(value)));
        let adjacent_year =
            is_year(token) && previous.is_some_and(|value| is_quarter(value) || is_month(value));
        if explicit_date
            || numeric_relative
            || adjacent_year
            || is_quarter(token)
            || is_month(token)
            || is_unit(token)
            || is_cue(token)
        {
            continue;
        }
        if !raw[index].trim().is_empty() {
            keep.push(raw[index]);
        }
    }
    if keep.is_empty() {
        query.to_string()
    } else {
        keep.join(" ")
    }
}

fn context_id(query: &str, mode: &str, hits: &[synapse_core::Hit]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(query.as_bytes());
    hasher.update(mode.as_bytes());
    for h in hits.iter().take(16) {
        hasher.update(&h.id.to_le_bytes());
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    hasher.update(&nonce.to_le_bytes());
    hasher.update(&std::process::id().to_le_bytes());
    hasher.finalize().to_hex()[..16].to_string()
}

fn print_context_pack(output: &ContextOutput<'_>) {
    println!("# Synapse Agent Memory Context Pack");
    println!();
    println!("context_id: {}", output.context_id);
    println!("query: {}", output.query);
    println!("mode: {}", output.mode);
    println!("route: {}", output.route);
    println!("budget: {}/{} chars", output.used_chars, output.budget);
    println!(
        "filters: noise={} superseded={} temporal={}",
        output.diagnostics.noise_filtered,
        output.diagnostics.superseded_filtered,
        output.diagnostics.temporal_filtered
    );
    if let (Some(lo), Some(hi)) = (
        &output.diagnostics.temporal_lo,
        &output.diagnostics.temporal_hi,
    ) {
        println!("event_window: {}..{}", lo, hi);
    }
    if let Some(retrieval_query) = &output.diagnostics.retrieval_query {
        println!("retrieval_query: {}", retrieval_query);
    }
    println!();
    println!("## Working brief");
    println!("- Use these memories as cited context, not unquestioned truth.");
    println!("- Prefer decision/fact/bugfix/benchmark memories over raw session notes.");
    println!(
        "- Pass: `synx feedback context:{} <doc_id> --gate pass --used <ids>`",
        output.context_id
    );
    println!(
        "- Fail: `synx feedback context:{} --gate fail`",
        output.context_id
    );
    println!("- If the task is freshness-sensitive, verify current docs before acting.");
    println!();
    println!("## Retrieved context");

    for h in output.hits {
        print!("{}", context_block(h));
    }

    println!();
    println!("## Fallback ladder");
    println!("1. Context above: lexical → hybrid → event/recent timeline");
    println!("2. `synx fallback <query>` when context is thin");
    println!("3. `synx fresh-context --prompt <query>` for package/API freshness");
    println!("4. `synx ground <query>` when graph expansion is useful");
}

fn print_context_json(output: &ContextOutput<'_>) -> Result<()> {
    let blocks: Vec<_> = output
        .hits
        .iter()
        .map(|h| {
            serde_json::json!({
                "id": h.id,
                "score": h.score,
                "title": h.title,
                "uri": h.uri,
                "kind": context_hit_kind(h),
                "priority": context_hit_priority(h),
                "captured_at": context_hit_captured_at(h),
                "occurred_at": context_hit_occurred_at(h),
                "text": h.text,
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "context_id": output.context_id,
            "query": output.query,
            "mode": output.mode,
            "budget_chars": output.budget,
            "used_chars": output.used_chars,
            "route": output.route,
            "retrieval": "lexical_then_hybrid_then_event_timeline",
            "filters": output.diagnostics,
            "hits": blocks,
            "reward_hint": {
                "pass": format!("synx feedback context:{} <doc_id> --gate pass --used <ids>", output.context_id),
                "fail": format!("synx feedback context:{} --gate fail", output.context_id)
            },
            "fallbacks": ["fallback", "fresh-context", "ground"]
        }))?
    );
    Ok(())
}

fn context_block(hit: &synapse_core::Hit) -> String {
    let title = hit.title.as_deref().unwrap_or("untitled");
    let uri = hit.uri.as_deref().unwrap_or("local:synapse");
    let captured = context_hit_captured_at(hit).unwrap_or_else(|| "unknown".to_string());
    let occurred = context_hit_occurred_at(hit).unwrap_or_else(|| "unspecified".to_string());
    format!(
        "\n### [{}] {} score={:.4}\nsource: {}\nkind: {} priority: {}\ncaptured_at: {} occurred_at: {}\n{}\n",
        hit.id,
        title,
        hit.score,
        uri,
        context_hit_kind(hit),
        context_hit_priority(hit),
        captured,
        occurred,
        hit.text
    )
}

fn bounded_context_hits(
    hits: &[synapse_core::Hit],
    budget: usize,
) -> (Vec<synapse_core::Hit>, usize) {
    let mut selected = Vec::new();
    let mut used = 0usize;
    for hit in hits {
        let mut candidate = hit.clone();
        candidate.text = compact(&candidate.text, 620);
        let full_cost = context_block(&candidate).len();
        if used + full_cost <= budget {
            used += full_cost;
            selected.push(candidate);
            continue;
        }
        let mut empty = candidate.clone();
        empty.text.clear();
        let overhead = context_block(&empty).len();
        let available = budget.saturating_sub(used + overhead);
        if available < 32 {
            break;
        }
        candidate.text = compact(&candidate.text, available);
        let cost = context_block(&candidate).len();
        if used + cost <= budget {
            used += cost;
            selected.push(candidate);
        }
        break;
    }
    (selected, used)
}

fn normalized_context_scores(hits: &[synapse_core::Hit]) -> Vec<f64> {
    if hits.is_empty() {
        return Vec::new();
    }
    let min = hits
        .iter()
        .map(|hit| hit.score)
        .fold(f64::INFINITY, f64::min);
    let max = hits
        .iter()
        .map(|hit| hit.score)
        .fold(f64::NEG_INFINITY, f64::max);
    let spread = max - min;
    hits.iter()
        .map(|hit| {
            if spread.is_finite() && spread > f64::EPSILON {
                ((hit.score - min) / spread).clamp(0.0, 1.0)
            } else {
                0.5
            }
        })
        .collect()
}

fn normalize_kind(kind: &str) -> Option<String> {
    match kind.trim().to_ascii_lowercase().as_str() {
        "decision" | "fact" | "preference" | "bugfix" | "benchmark" | "command" | "session"
        | "adr" | "research" | "note" => Some(kind.trim().to_ascii_lowercase()),
        _ => None,
    }
}

fn confidence_score(confidence: &str) -> Option<f64> {
    match confidence.trim().to_ascii_lowercase().as_str() {
        "high" => Some(0.90),
        "medium" => Some(0.70),
        "low" => Some(0.50),
        _ => None,
    }
}

fn normalize_priority(priority: &str) -> Option<String> {
    match priority.trim().to_ascii_lowercase().as_str() {
        value @ ("critical" | "high" | "normal" | "low") => Some(value.to_string()),
        _ => None,
    }
}

fn normalize_freshness(freshness: &str) -> Option<String> {
    match freshness.trim().to_ascii_lowercase().as_str() {
        value @ ("stable" | "slow" | "fast" | "volatile") => Some(value.to_string()),
        _ => None,
    }
}

fn memory_type_for_kind(kind: &str) -> MemoryType {
    match kind {
        "decision" | "adr" => MemoryType::Decision,
        "fact" | "benchmark" | "research" => MemoryType::Fact,
        "preference" => MemoryType::Preference,
        "bugfix" => MemoryType::Lesson,
        "command" | "session" => MemoryType::Episodic,
        _ => MemoryType::Raw,
    }
}

fn parse_doc_ids(value: &str) -> Result<Vec<i64>> {
    let mut ids = Vec::new();
    for token in value
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        let id = token
            .parse::<i64>()
            .with_context(|| format!("invalid docs.id in --used: {token}"))?;
        anyhow::ensure!(id > 0, "docs.id values in --used must be positive");
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    anyhow::ensure!(!ids.is_empty(), "--used must contain at least one docs.id");
    Ok(ids)
}

#[allow(clippy::too_many_arguments)]
fn merge_remember_metadata(
    store: &Store,
    doc_id: i64,
    kind: &str,
    freshness: &str,
    confidence: &str,
    confidence_score: f64,
    priority: &str,
    captured_at: i64,
    occurred_at: Option<&str>,
    occurred_ts: Option<i64>,
) -> Result<()> {
    let mut meta = store
        .get(doc_id)?
        .meta
        .unwrap_or_else(|| serde_json::json!({}));
    let object = meta
        .as_object_mut()
        .context("stored document metadata is not a JSON object")?;
    object.insert("kind".into(), serde_json::json!(kind));
    object.insert("freshness".into(), serde_json::json!(freshness));
    object.insert("confidence".into(), serde_json::json!(confidence));
    object.insert(
        "confidence_score".into(),
        serde_json::json!(confidence_score),
    );
    object.insert("priority".into(), serde_json::json!(priority));
    object.insert("observed_at".into(), serde_json::json!(captured_at));
    object.insert("source".into(), serde_json::json!("synx remember"));
    object.insert("chunker".into(), serde_json::json!("synx-cli-v1.1"));
    if let Some(value) = occurred_at {
        object.insert("occurred_at".into(), serde_json::json!(value));
    }
    if let Some(value) = occurred_ts {
        object.insert("occurred_ts".into(), serde_json::json!(value));
    }
    store.conn.execute(
        "UPDATE docs SET meta=?1 WHERE id=?2",
        rusqlite::params![meta.to_string(), doc_id],
    )?;
    Ok(())
}

fn auto_title(kind: &str, text: &str) -> String {
    let slug = text
        .split_whitespace()
        .take(8)
        .map(|w| {
            w.chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect::<String>()
                .to_ascii_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!(
        "{}:{}",
        kind,
        if slug.is_empty() { "memory" } else { &slug }
    )
}

fn compact(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut out = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(trimmed);
        if out.len() >= max_chars {
            break;
        }
    }
    if out.len() > max_chars {
        let ellipsis = '…';
        let mut end = max_chars.saturating_sub(ellipsis.len_utf8());
        while end > 0 && !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
        if max_chars >= ellipsis.len_utf8() {
            out.push(ellipsis);
        }
    }
    out
}

#[derive(Debug, Clone, serde::Serialize)]
struct RepairReport {
    backup_path: String,
    backup_verified: bool,
    action: String,
    docs: i64,
    fts_rows_before: i64,
    fts_rows_after: i64,
    quick_check_after: String,
}

#[derive(serde::Serialize)]
struct DoctorReport {
    db: String,
    health: String,
    quick_check: String,
    semantic_enabled: bool,
    docs: i64,
    vectors: i64,
    fts_rows: i64,
    fts_mismatch: i64,
    duplicate_hash_groups: i64,
    missing_vectors: i64,
    private_source_hits: i64,
    stale_or_generated_source_hits: i64,
    context_noise_hits: i64,
    active_raw_memories: i64,
    pending_extractions: i64,
    event_dated_memories: i64,
    superseded_memories: i64,
    incomplete_repairs: i64,
    embed_cache: Option<String>,
    backup_path: Option<String>,
    backup_age_seconds: Option<i64>,
    fallbacks: Vec<&'static str>,
    warnings: Vec<String>,
    repair: Option<RepairReport>,
}

fn doctor_report(store: &Store, file: &std::path::Path) -> Result<DoctorReport> {
    let semantic_enabled = cfg!(any(feature = "static-ort", feature = "cross-linux"));
    let stats = store.stats()?;
    let quick_check = store
        .conn
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .unwrap_or_else(|_| "failed".to_string());
    let duplicate_hash_groups = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM (SELECT blake3 FROM docs GROUP BY blake3 HAVING COUNT(*) > 1)",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let fts_rows = fts_indexed_rows(store).unwrap_or(-1);
    let fts_mismatch = if fts_rows < 0 {
        stats.docs
    } else {
        (stats.docs - fts_rows).abs()
    };
    let missing_vectors = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM docs d LEFT JOIN docs_vec v ON d.id = v.id WHERE v.id IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let private_source_hits = doctor_source_count(
        store,
        &[
            "/.claude/projects",
            "/.codex/sessions",
            "/file-history/",
            "/node_modules/",
            "/.trash/",
        ],
    );
    let stale_or_generated_source_hits = doctor_source_count(
        store,
        &[
            "\"status\":\"stale\"",
            "\"status\": \"stale\"",
            "/generated/",
            "/generated-src/",
            "/target/",
            "/dist/",
            "/build/",
            "/node_modules/",
            "/file-history/",
        ],
    );
    let context_noise_hits = doctor_context_noise_count(store);
    let active_raw_memories = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE memory_type='raw' AND superseded_by IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let pending_extractions = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM extraction_queue WHERE status='pending'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let event_dated_memories = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE event_date IS NOT NULL AND event_date != ''",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let superseded_memories = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE superseded_by IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let incomplete_repairs = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM health_events WHERE status != 'ok'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let embed_cache = semantic_enabled
        .then(|| file.parent().map(|p| p.join(".emb-cache")))
        .flatten()
        .filter(|p| p.exists())
        .map(|p| p.display().to_string());
    let backup = newest_backup(file);
    let mut warnings = Vec::new();
    if quick_check != "ok" {
        warnings.push("sqlite quick_check failed".to_string());
    }
    if duplicate_hash_groups > 0 {
        warnings.push("duplicate hash groups detected".to_string());
    }
    if fts_mismatch > 0 {
        warnings.push("FTS5 row count differs from canonical docs; run doctor --fix".to_string());
    }
    if semantic_enabled && missing_vectors > 0 {
        warnings.push("docs without vectors: run import/re-embed path when available".to_string());
    }
    if private_source_hits > 0 {
        warnings.push(
            "private/session source paths detected; quarantine or re-import clean sources"
                .to_string(),
        );
    }
    if stale_or_generated_source_hits > 0 {
        warnings.push(
            "stale/generated source paths detected; prefer curated docs or source manifests"
                .to_string(),
        );
    }
    if context_noise_hits > 0 {
        warnings.push(
            "transport/status noise exists; context filters it without deleting source data"
                .to_string(),
        );
    }
    if pending_extractions > 0 {
        warnings.push(
            "raw memories await typing; use synx remember for high-value durable truth".to_string(),
        );
    }
    if incomplete_repairs > 0 {
        warnings.push("an earlier repair did not reach its verified end state".to_string());
    }
    if semantic_enabled && embed_cache.is_none() {
        warnings.push("embedding cache missing; first semantic query may be slow".to_string());
    }
    if stats.docs > 0 && backup.is_none() {
        warnings.push("no .synx/.brainpack backup found next to db or in backups/".to_string());
    }
    if let Some((_, age)) = &backup
        && *age > 7 * 24 * 60 * 60
    {
        warnings.push("latest backup is older than 7 days".to_string());
    }
    let (backup_path, backup_age_seconds) = backup
        .map(|(path, age)| (Some(path), Some(age)))
        .unwrap_or((None, None));
    let health = if quick_check != "ok" {
        "unsafe"
    } else if fts_mismatch > 0 {
        "repairable"
    } else if warnings.is_empty() {
        "healthy"
    } else {
        "attention"
    }
    .to_string();
    Ok(DoctorReport {
        db: file.display().to_string(),
        health,
        quick_check,
        semantic_enabled,
        docs: stats.docs,
        vectors: stats.vecs,
        fts_rows,
        fts_mismatch,
        duplicate_hash_groups,
        missing_vectors,
        private_source_hits,
        stale_or_generated_source_hits,
        context_noise_hits,
        active_raw_memories,
        pending_extractions,
        event_dated_memories,
        superseded_memories,
        incomplete_repairs,
        embed_cache,
        backup_path,
        backup_age_seconds,
        fallbacks: if semantic_enabled {
            vec!["hybrid", "lexical", "timeline", "fresh-context", "ground"]
        } else {
            vec!["lexical", "timeline", "fresh-context", "ground"]
        },
        warnings,
        repair: None,
    })
}

fn print_doctor_report(report: &DoctorReport) {
    println!("# Synapse Agent Memory doctor");
    println!(
        "db={} health={} quick_check={}",
        report.db, report.health, report.quick_check
    );
    println!("semantic_enabled={}", report.semantic_enabled);
    println!(
        "docs={} vectors={} fts_rows={} fts_mismatch={}",
        report.docs, report.vectors, report.fts_rows, report.fts_mismatch
    );
    println!("duplicate_hash_groups={}", report.duplicate_hash_groups);
    println!("missing_vectors={}", report.missing_vectors);
    println!("private_source_hits={}", report.private_source_hits);
    println!(
        "stale_or_generated_source_hits={}",
        report.stale_or_generated_source_hits
    );
    println!("context_noise_hits={}", report.context_noise_hits);
    println!("active_raw_memories={}", report.active_raw_memories);
    println!("pending_extractions={}", report.pending_extractions);
    println!("event_dated_memories={}", report.event_dated_memories);
    println!("superseded_memories={}", report.superseded_memories);
    println!("incomplete_repairs={}", report.incomplete_repairs);
    println!(
        "embed_cache={}",
        report.embed_cache.as_deref().unwrap_or("missing")
    );
    println!(
        "backup={}",
        report.backup_path.as_deref().unwrap_or("missing")
    );
    println!(
        "backup_age_seconds={}",
        report
            .backup_age_seconds
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    );
    println!("fallbacks={}", report.fallbacks.join(","));
    for warning in &report.warnings {
        println!("warning={}", warning);
    }
    if let Some(repair) = &report.repair {
        println!("repair_action={}", repair.action);
        println!("repair_backup={}", repair.backup_path);
        println!("repair_backup_verified={}", repair.backup_verified);
        println!("repair_quick_check_after={}", repair.quick_check_after);
    }
}

fn safe_repair(store: &Store, file: &std::path::Path) -> Result<RepairReport> {
    let quick_check: String = store
        .conn
        .query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    anyhow::ensure!(
        quick_check == "ok",
        "refusing repair: canonical SQLite quick_check is {quick_check}"
    );
    let parent = file
        .parent()
        .context("brain file has no parent directory")?;
    let backup_dir = parent.join("backups");
    std::fs::create_dir_all(&backup_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&backup_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let stem = file
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("brain");
    let backup = backup_dir.join(format!("{stem}.pre-repair-{}.brainpack", now_ms()));
    snap::export(file, &backup, 3).context("create pre-repair brainpack")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&backup, std::fs::Permissions::from_mode(0o600))?;
    }

    let verify_dir = tempfile::tempdir()?;
    let verify_db = verify_dir.path().join("verify.db");
    snap::import(&backup, &verify_db).context("verify pre-repair brainpack hash")?;
    let verify_conn = rusqlite::Connection::open(&verify_db)?;
    let backup_quick_check: String =
        verify_conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    anyhow::ensure!(
        backup_quick_check == "ok",
        "refusing repair: backup quick_check is {backup_quick_check}"
    );

    store.conn.execute(
        "INSERT INTO health_events(ts,event_kind,status,details_json)
         VALUES(?1,'safe_repair','started',?2)",
        rusqlite::params![
            now_secs(),
            serde_json::json!({
                "backup_path": backup.display().to_string(),
                "backup_verified": true
            })
            .to_string()
        ],
    )?;
    let health_event_id = store.conn.last_insert_rowid();

    let docs: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM docs", [], |row| row.get(0))?;
    let fts_rows_before = fts_indexed_rows(store)?;
    let action = if fts_rows_before != docs {
        store
            .conn
            .execute_batch("INSERT INTO docs_fts(docs_fts) VALUES('rebuild');")?;
        "fts_rebuild"
    } else {
        store
            .conn
            .execute_batch("INSERT INTO docs_fts(docs_fts) VALUES('optimize');")?;
        "fts_optimize"
    };
    store
        .conn
        .execute_batch("INSERT INTO docs_fts(docs_fts) VALUES('integrity-check');")?;
    let fts_rows_after = fts_indexed_rows(store)?;
    anyhow::ensure!(
        fts_rows_after == docs,
        "FTS repair incomplete: docs={docs} fts_rows={fts_rows_after}"
    );
    let quick_check_after: String = store
        .conn
        .query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    anyhow::ensure!(
        quick_check_after == "ok",
        "post-repair quick_check failed: {quick_check_after}"
    );
    let report = RepairReport {
        backup_path: backup.display().to_string(),
        backup_verified: true,
        action: action.to_string(),
        docs,
        fts_rows_before,
        fts_rows_after,
        quick_check_after,
    };
    store.conn.execute(
        "UPDATE health_events SET status='ok', details_json=?1 WHERE id=?2",
        rusqlite::params![serde_json::to_string(&report)?, health_event_id],
    )?;
    Ok(report)
}

fn fts_indexed_rows(store: &Store) -> Result<i64> {
    // External-content FTS tables mirror COUNT(*) from `docs`; the docsize
    // shadow table reflects actual indexed rows and therefore detects drift.
    store
        .conn
        .query_row("SELECT COUNT(*) FROM docs_fts_docsize", [], |row| {
            row.get(0)
        })
        .map_err(Into::into)
}

fn doctor_source_count(store: &Store, needles: &[&str]) -> i64 {
    let mut clauses = Vec::new();
    for needle in needles {
        let escaped = needle.replace('\'', "''").to_ascii_lowercase();
        clauses.push(format!("haystack LIKE '%{}%'", escaped));
    }
    let sql = format!(
        "SELECT COUNT(*) FROM (
            SELECT lower(coalesce(uri,'') || ' ' || coalesce(title,'') || ' ' || coalesce(meta,'')) AS haystack
            FROM docs
        ) WHERE {}",
        clauses.join(" OR ")
    );
    store.conn.query_row(&sql, [], |r| r.get(0)).unwrap_or(0)
}

fn doctor_context_noise_count(store: &Store) -> i64 {
    store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM docs
             WHERE lower(ltrim(text)) LIKE '[telepathy]%'
                OR lower(text) LIKE '%<task-notification>%'
                OR lower(text) LIKE '%tool-use-id%'
                OR (json_valid(meta) AND lower(coalesce(json_extract(meta,'$.status'),''))
                    IN ('stale','archived','noise','generated'))
                OR (json_valid(meta) AND lower(coalesce(json_extract(meta,'$.kind'),''))
                    IN ('status','telepathy','notification'))",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0)
}

fn newest_backup(file: &std::path::Path) -> Option<(String, i64)> {
    let parent = file.parent()?;
    let mut dirs = vec![parent.to_path_buf()];
    dirs.push(parent.join("backups"));
    let mut newest: Option<(std::path::PathBuf, std::time::SystemTime)> = None;
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !is_backup_path(&path) {
                continue;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let Ok(modified) = meta.modified() else {
                continue;
            };
            if newest.as_ref().map(|(_, t)| modified > *t).unwrap_or(true) {
                newest = Some((path, modified));
            }
        }
    }
    newest.map(|(path, modified)| {
        let age = std::time::SystemTime::now()
            .duration_since(modified)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        (path.display().to_string(), age)
    })
}

fn is_backup_path(path: &std::path::Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("synx") | Some("brainpack") | Some("bp")
    )
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(id: i64, text: &str, meta: serde_json::Value) -> synapse_core::Hit {
        synapse_core::Hit {
            id,
            uri: Some(format!("local:{id}")),
            title: Some(format!("doc-{id}")),
            text: text.to_string(),
            score: 0.5,
            meta: Some(meta),
            ts: Some(1_783_000_000_000),
        }
    }

    #[test]
    fn fresh_input_accepts_hook_json() {
        let (prompt, cwd, project) = parse_fresh_input(
            r#"{"prompt":"latest serde API","cwd":"/tmp/demo","project":"demo"}"#,
        );
        assert_eq!(prompt, "latest serde API");
        assert_eq!(cwd, Some(PathBuf::from("/tmp/demo")));
        assert_eq!(project.as_deref(), Some("demo"));
    }

    #[test]
    fn fresh_input_keeps_plain_prompt() {
        let (prompt, cwd, project) = parse_fresh_input(" install newest better-sqlite3 ");
        assert_eq!(prompt, "install newest better-sqlite3");
        assert!(cwd.is_none());
        assert!(project.is_none());
    }

    #[test]
    fn context_noise_filter_is_narrow_and_explicit() {
        assert!(is_context_noise(&hit(
            1,
            "[telepathy] models_loaded status",
            serde_json::json!({"kind":"note"})
        )));
        assert!(is_context_noise(&hit(
            2,
            "useful old note",
            serde_json::json!({"status":"archived"})
        )));
        assert!(!is_context_noise(&hit(
            3,
            "Decision: Telepathy transport stays optional",
            serde_json::json!({"kind":"decision","priority":"high"})
        )));
    }

    #[test]
    fn temporal_cues_do_not_poison_lexical_query() {
        assert_eq!(
            temporal_retrieval_query("Portable Synapse truth Q3 2026"),
            "Portable Synapse truth"
        );
        assert_eq!(
            temporal_retrieval_query("Welche Entscheidung vor 3 Tagen?"),
            "Welche Entscheidung"
        );
        assert_eq!(
            temporal_retrieval_query("release on 2026-07-14"),
            "release on"
        );
    }

    #[test]
    fn context_budget_is_hard_cap() {
        let source = hit(
            1,
            &"memory ".repeat(500),
            serde_json::json!({"kind":"decision","priority":"critical"}),
        );
        let (selected, used) = bounded_context_hits(&[source], 420);
        assert_eq!(selected.len(), 1);
        assert!(used <= 420);
        assert!(selected[0].text.len() < 500 * "memory ".len());
    }

    #[test]
    fn priority_and_supersession_change_context_rank() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut store = Store::open(tmp.path()).unwrap();
        let old = store
            .put(&PutRequest {
                text: "old decision".into(),
                meta: Some(serde_json::json!({"kind":"decision","priority":"critical"})),
                ..Default::default()
            })
            .unwrap();
        let normal = store
            .put(&PutRequest {
                text: "normal decision".into(),
                meta: Some(serde_json::json!({"kind":"decision","priority":"normal"})),
                ..Default::default()
            })
            .unwrap();
        let high = store
            .put(&PutRequest {
                text: "high decision".into(),
                meta: Some(serde_json::json!({"kind":"decision","priority":"high"})),
                ..Default::default()
            })
            .unwrap();
        promote_doc_memory(&store.conn, old, MemoryType::Decision, 0.9, None, None).unwrap();
        promote_doc_memory(
            &store.conn,
            high,
            MemoryType::Decision,
            0.9,
            None,
            Some(old),
        )
        .unwrap();
        let ranked = rank_context_hits(
            &store,
            None,
            vec![
                hit(
                    old,
                    "old decision",
                    serde_json::json!({"kind":"decision","priority":"critical"}),
                ),
                hit(
                    normal,
                    "normal decision",
                    serde_json::json!({"kind":"decision","priority":"normal"}),
                ),
                hit(
                    high,
                    "high decision",
                    serde_json::json!({"kind":"decision","priority":"high"}),
                ),
            ],
        )
        .unwrap();
        assert_eq!(
            ranked.iter().map(|value| value.id).collect::<Vec<_>>(),
            vec![high, normal]
        );
    }

    #[test]
    fn safe_repair_backs_up_before_rebuilding_fts() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("brain.db");
        let mut store = Store::open(&db).unwrap();
        let id = store
            .put(&PutRequest {
                text: "repairable search index".into(),
                ..Default::default()
            })
            .unwrap();
        store
            .conn
            .execute("DELETE FROM docs_fts WHERE rowid=?1", [id])
            .unwrap();

        let report = safe_repair(&store, &db).unwrap();
        assert_eq!(report.action, "fts_rebuild");
        assert!(report.backup_verified);
        assert!(std::path::Path::new(&report.backup_path).exists());
        assert_eq!(report.docs, report.fts_rows_after);
        let events: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM health_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(events, 1);
    }
}
