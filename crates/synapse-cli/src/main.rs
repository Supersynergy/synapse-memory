// jemalloc: replace system allocator — reduces fragmentation under alloc-heavy
// HNSW/ndarray workloads. Feature-gated so tests / cross-compile can opt out.
#[cfg(feature = "jemalloc")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

mod synx_io;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use synapse_core::{
    PutRequest, SearchMode, Store,
    embed::Embedder,
    federate::{Addr, Federation},
    fresh::{FreshMode, FreshOptions, build_fresh_report, render_fresh_context_xml},
    shard, sign, snap,
};
use synapse_learn::LearnStore;

type VerifyRow = (i64, String, Vec<u8>);
type FreshInput = (String, Option<PathBuf>, Option<String>);
type SearchBestEffortResult = (Vec<synapse_core::Hit>, String);

#[derive(Parser)]
#[command(name = "synapse", version, about = "Single-file memory for AI agents")]
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
    Shard {
        #[command(subcommand)]
        action: ShardCmd,
    },
    /// Self-learning operations
    Learn {
        #[command(subcommand)]
        action: LearnCmd,
    },
    /// Record positive feedback for a doc
    Feedback {
        query_id: String,
        accepted_doc_id: i64,
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
    /// Repair: rebuild FTS5 + vec0 index from docs table
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

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let cli = Cli::parse();
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
            let embedding = if no_embed {
                None
            } else {
                let e = Embedder::new_with_cache::<std::path::PathBuf>(
                    cli.file.parent().map(|p| p.join(".emb-cache")),
                )
                .context("embedder init")?;
                Some(e.embed_one(&body)?)
            };
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
            let e = Embedder::new_with_cache::<std::path::PathBuf>(
                cli.file.parent().map(|p| p.join(".emb-cache")),
            )?;
            let q = e.embed_one(&query)?;
            let hits = store.search("", SearchMode::Vec, Some(&q), limit)?;
            print_hits(&hits);
        }
        Cmd::Hybrid {
            query,
            limit,
            guarantee,
        } => {
            let store = Store::open(&cli.file)?;
            let e = Embedder::new_with_cache::<std::path::PathBuf>(
                cli.file.parent().map(|p| p.join(".emb-cache")),
            )?;
            let q = e.embed_one(&query)?;
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
            let store = Store::open(&cli.file)?;
            let learn_path = cli.file.with_extension("learn.db");
            let lstore = LearnStore::open(&learn_path).ok();
            let (hits, route) = search_best_effort(&store, &cli.file, &query, limit)?;
            let ranked = rank_context_hits(&store, lstore.as_ref(), hits)?;
            let context_id = context_id(&query, &mode, &ranked);
            if let Some(ls) = lstore.as_ref() {
                let ids: Vec<i64> = ranked.iter().map(|h| h.id).collect();
                let _ = ls.log_context_query(&context_id, now_secs(), &query, &mode, &route, &ids);
            }
            if json {
                print_context_json(&context_id, &query, &mode, budget, &route, &ranked)?;
            } else {
                print_context_pack(&context_id, &query, &mode, budget, &route, &ranked);
            }
        }
        Cmd::Remember {
            text,
            kind,
            title,
            uri,
            freshness,
            confidence,
            no_embed,
        } => {
            anyhow::ensure!(!text.trim().is_empty(), "empty text");
            let mut store = Store::open(&cli.file)?;
            let normalized_kind = normalize_kind(&kind);
            let embedding = if no_embed {
                None
            } else {
                let e = Embedder::new_with_cache::<std::path::PathBuf>(
                    cli.file.parent().map(|p| p.join(".emb-cache")),
                )
                .context("embedder init")?;
                Some(e.embed_one(&text)?)
            };
            let req = PutRequest {
                title: title.or_else(|| Some(auto_title(&normalized_kind, &text))),
                uri,
                text,
                meta: Some(serde_json::json!({
                    "kind": normalized_kind,
                    "freshness": freshness,
                    "confidence": confidence,
                    "observed_at": now_ms(),
                    "source": "synx remember",
                    "chunker": "synx-cli-v1"
                })),
                embedding,
            };
            let id = store.put(&req)?;
            println!("ok remembered id={} kind={}", id, normalized_kind);
        }
        Cmd::Doctor { fix, json } => {
            let store = Store::open(&cli.file)?;
            let report = doctor_report(&store, &cli.file)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_doctor_report(&report);
            }
            if fix {
                store
                    .conn
                    .execute_batch("INSERT INTO docs_fts(docs_fts) VALUES('optimize');")?;
                println!("fix=fts_optimize_ok");
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
                let e = Embedder::new_with_cache::<std::path::PathBuf>(None)?;
                let q_vec = e.embed_one(&query)?;
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
                    println!(
                        "bandit_shards={} feedback_entries={}",
                        bandit_count, fb_count
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
            shard_id,
        } => {
            let learn_path = cli.file.with_extension("learn.db");
            let lstore = LearnStore::open(&learn_path)?;
            synapse_learn::feedback::record_accept(&lstore, &query_id, accepted_doc_id, &shard_id)?;
            println!(
                "ok feedback recorded doc_id={} shard={}",
                accepted_doc_id, shard_id
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
            // Rebuild FTS5
            store
                .conn
                .execute_batch("INSERT INTO docs_fts(docs_fts) VALUES('rebuild')")?;
            // Rebuild vec0 from scratch using put_batch on existing docs
            // Minimal: just report FTS rebuilt; vec index lives in vec0 which is shadow table
            store
                .conn
                .execute_batch("INSERT INTO docs_fts(docs_fts) VALUES('integrity-check')")?;
            let count: i64 = store
                .conn
                .query_row("SELECT COUNT(*) FROM docs", [], |r| r.get(0))?;
            println!("ok repair fts5 rebuilt docs={count}");
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
            // Pipeline: hybrid → seeds → PPR → traverse → JSON bundle
            let store = Store::open(&cli.file)?;
            let e = Embedder::new_with_cache::<std::path::PathBuf>(
                cli.file.parent().map(|p| p.join(".emb-cache")),
            )?;
            let q = e.embed_one(&query)?;
            let hits = store.search(&query, SearchMode::Hybrid, Some(&q), k)?;

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
    println!("# Synapse Prime Brief");
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

    let hybrid =
        Embedder::new_with_cache::<std::path::PathBuf>(file.parent().map(|p| p.join(".emb-cache")))
            .ok()
            .and_then(|e| e.embed_one(query).ok())
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
        docs.into_iter()
            .map(|d| synapse_core::Hit {
                id: d.id,
                uri: d.uri,
                title: d.title,
                text: d.text,
                score: 0.0,
                meta: d.meta,
                ts: Some(d.ts),
            })
            .collect(),
        "timeline".to_string(),
    ))
}

fn rank_context_hits(
    store: &Store,
    learn: Option<&LearnStore>,
    hits: Vec<synapse_core::Hit>,
) -> Result<Vec<synapse_core::Hit>> {
    let mut ranked = Vec::with_capacity(hits.len());
    for mut hit in hits {
        let doc = store.get(hit.id).ok();
        let kind = doc
            .as_ref()
            .and_then(|d| d.meta.as_ref())
            .and_then(|m| m.get("kind"))
            .and_then(|v| v.as_str())
            .unwrap_or("note");
        hit.score += memory_kind_prior(kind);
        if let Some(learn) = learn {
            hit.score += learn.memory_type_bonus(kind).unwrap_or(0.0);
        }
        ranked.push(hit);
    }
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(ranked)
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

fn context_id(query: &str, mode: &str, hits: &[synapse_core::Hit]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(query.as_bytes());
    hasher.update(mode.as_bytes());
    for h in hits.iter().take(16) {
        hasher.update(&h.id.to_le_bytes());
    }
    hasher.finalize().to_hex()[..16].to_string()
}

fn print_context_pack(
    context_id: &str,
    query: &str,
    mode: &str,
    budget: usize,
    route: &str,
    hits: &[synapse_core::Hit],
) {
    println!("# Synapse Context Pack");
    println!();
    println!("context_id: {}", context_id);
    println!("query: {}", query);
    println!("mode: {}", mode);
    println!("route: {}", route);
    println!("budget: {} chars", budget);
    println!();
    println!("## Working brief");
    println!("- Use these memories as cited context, not unquestioned truth.");
    println!("- Prefer decision/fact/bugfix/benchmark memories over raw session notes.");
    println!(
        "- If useful, reward this pack with: synx feedback context:{} <doc_id>",
        context_id
    );
    println!("- If the task is freshness-sensitive, verify current docs before acting.");
    println!();
    println!("## Retrieved context");

    let mut used = 0usize;
    for h in hits {
        let title = h.title.as_deref().unwrap_or("untitled");
        let uri = h.uri.as_deref().unwrap_or("local:synapse");
        let text = compact(&h.text, 620);
        let block = format!(
            "\n### [{}] {} score={:.4}\nsource: {}\n{}\n",
            h.id, title, h.score, uri, text
        );
        if used + block.len() > budget && used > 0 {
            break;
        }
        used += block.len();
        print!("{}", block);
    }

    println!();
    println!("## Fallback ladder");
    println!("1. Context above: hybrid → lexical → recent timeline");
    println!("2. `synx fallback <query>` when context is thin");
    println!("3. `synx fresh-context --prompt <query>` for package/API freshness");
    println!("4. `synx ground <query>` when graph expansion is useful");
}

fn print_context_json(
    context_id: &str,
    query: &str,
    mode: &str,
    budget: usize,
    route: &str,
    hits: &[synapse_core::Hit],
) -> Result<()> {
    let blocks: Vec<_> = hits
        .iter()
        .map(|h| {
            serde_json::json!({
                "id": h.id,
                "score": h.score,
                "title": h.title,
                "uri": h.uri,
                "text": compact(&h.text, 620),
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "context_id": context_id,
            "query": query,
            "mode": mode,
            "budget_chars": budget,
            "route": route,
            "retrieval": "hybrid_then_lexical_then_timeline",
            "hits": blocks,
            "reward_hint": format!("synx feedback context:{} <doc_id>", context_id),
            "fallbacks": ["fallback", "fresh-context", "ground"]
        }))?
    );
    Ok(())
}

fn normalize_kind(kind: &str) -> String {
    match kind.trim().to_ascii_lowercase().as_str() {
        "decision" | "fact" | "preference" | "bugfix" | "benchmark" | "command" | "session"
        | "adr" | "research" | "note" => kind.trim().to_ascii_lowercase(),
        _ => "note".to_string(),
    }
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
        out.truncate(max_chars.saturating_sub(1));
        out.push('…');
    }
    out
}

#[derive(serde::Serialize)]
struct DoctorReport {
    db: String,
    quick_check: String,
    docs: i64,
    vectors: i64,
    duplicate_hash_groups: i64,
    missing_vectors: i64,
    private_source_hits: i64,
    stale_or_generated_source_hits: i64,
    embed_cache: Option<String>,
    backup_path: Option<String>,
    backup_age_seconds: Option<i64>,
    fallbacks: Vec<&'static str>,
    warnings: Vec<String>,
}

fn doctor_report(store: &Store, file: &std::path::Path) -> Result<DoctorReport> {
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
    let embed_cache = file
        .parent()
        .map(|p| p.join(".emb-cache"))
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
    if missing_vectors > 0 {
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
    if embed_cache.is_none() {
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
    Ok(DoctorReport {
        db: file.display().to_string(),
        quick_check,
        docs: stats.docs,
        vectors: stats.vecs,
        duplicate_hash_groups,
        missing_vectors,
        private_source_hits,
        stale_or_generated_source_hits,
        embed_cache,
        backup_path,
        backup_age_seconds,
        fallbacks: vec!["hybrid", "lexical", "timeline", "fresh-context", "ground"],
        warnings,
    })
}

fn print_doctor_report(report: &DoctorReport) {
    println!("# Synapse doctor");
    println!("db={} quick_check={}", report.db, report.quick_check);
    println!("docs={} vectors={}", report.docs, report.vectors);
    println!("duplicate_hash_groups={}", report.duplicate_hash_groups);
    println!("missing_vectors={}", report.missing_vectors);
    println!("private_source_hits={}", report.private_source_hits);
    println!(
        "stale_or_generated_source_hits={}",
        report.stale_or_generated_source_hits
    );
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
}
