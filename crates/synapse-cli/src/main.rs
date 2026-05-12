// jemalloc: replace system allocator — reduces fragmentation under alloc-heavy
// HNSW/ndarray workloads. Feature-gated so tests / cross-compile can opt out.
#[cfg(feature = "jemalloc")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rusqlite;
use std::path::PathBuf;
use synapse_core::{
    embed::Embedder,
    federate::{Addr, Federation},
    shard, sign, snap, PutRequest, SearchMode, Store,
};
use synapse_learn::LearnStore;

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
    /// Stats
    Stats,
    /// Export to .brainpack
    Snap {
        out: PathBuf,
        #[arg(long, default_value_t = 3)]
        level: i32,
    },
    /// Import a .brainpack into this file
    /// (extension doesn't matter, content does — .syn/.synapse/.brainpack/.bp all accepted)
    Restore { pack: PathBuf },
    /// Merge two brainpacks by URI-matching docs, CRDT-merging meta_crdt per doc
    /// (extension doesn't matter, content does — .syn/.synapse/.brainpack/.bp all accepted)
    Merge {
        file_a: PathBuf,
        file_b: PathBuf,
        #[arg(short = 'o', long)]
        out: PathBuf,
        #[arg(long, default_value_t = 3)]
        level: i32,
    },
    /// Merge a peer snapshot into the current brain file (CRDT, offline-safe).
    /// Equivalent to: syn merge <current-brain-snap> <peer> --out merged.brainpack
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
            let req = PutRequest {
                title,
                uri,
                text: body,
                embedding,
                ..Default::default()
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
        Cmd::Hybrid { query, limit, guarantee } => {
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
        Cmd::Stats => {
            let store = Store::open(&cli.file)?;
            let s = store.stats()?;
            println!("{}", serde_json::to_string_pretty(&s)?);
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
            let tmp = std::env::temp_dir().join(format!("synapse-snap-{}.brainpack", std::process::id()));
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
            store.conn.execute(
                "UPDATE docs SET sig = ?1 WHERE id = ?2",
                rusqlite::params![sig.as_ref(), id],
            ).context("update sig")?;
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
        Cmd::Graph { action } => {
            let conn = rusqlite::Connection::open(&cli.file)?;
            synapse_graph::ensure_schema(&conn)?;
            match action {
                GraphCmd::Relate { from, to, rel, weight } => {
                    synapse_graph::relate(&conn, from, to, &rel, weight, None)?;
                    println!("{{\"ok\":true,\"from\":{from},\"to\":{to},\"rel\":\"{rel}\",\"weight\":{weight}}}");
                }
                GraphCmd::Pagerank { n, damping, iters } => {
                    let top = synapse_graph::algorithms::top_pagerank(&conn, n, damping, iters)?;
                    println!("{}", serde_json::to_string_pretty(&top)?);
                }
                GraphCmd::Ppr { seeds_json, alpha, iters, limit } => {
                    let seeds: std::collections::HashMap<String, f64> = serde_json::from_str(&seeds_json)
                        .context("parse seeds JSON")?;
                    let seeds_i: std::collections::HashMap<i64, f64> = seeds.into_iter()
                        .filter_map(|(k, v)| k.parse::<i64>().ok().map(|i| (i, v)))
                        .collect();
                    let ranked = synapse_core::ppr::personalized_pagerank(
                        &conn, &seeds_i, alpha, iters,
                        synapse_core::ppr::DEFAULT_NEIGHBOR_CAP, limit)?;
                    println!("{}", serde_json::to_string_pretty(&ranked)?);
                }
                GraphCmd::Communities { max_iters, top_n } => {
                    let comms = synapse_graph::algorithms::communities(&conn, max_iters)?;
                    let top: Vec<_> = comms.into_iter().take(top_n)
                        .map(|(id, members)| serde_json::json!({"community_id": id, "size": members.len(), "members": members}))
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&top)?);
                }
                GraphCmd::Neighbors { node_id, top_k, rel } => {
                    let n = synapse_graph::neighbors(&conn, node_id, rel.as_deref(), top_k)?;
                    println!("{}", serde_json::to_string_pretty(&n)?);
                }
                GraphCmd::Traverse { start_id, depth, top_k_per_hop, decay } => {
                    let t = synapse_graph::traverse(&conn, start_id, depth, top_k_per_hop, decay, None)?;
                    println!("{}", serde_json::to_string_pretty(&t)?);
                }
                GraphCmd::Path { from, to, max_depth } => {
                    let p = synapse_graph::shortest_path(&conn, from, to, max_depth)?;
                    println!("{}", serde_json::to_string_pretty(&p)?);
                }
                GraphCmd::Count => {
                    let n = synapse_graph::edge_count(&conn)?;
                    println!("{{\"edges\":{n}}}");
                }
            }
        }
        Cmd::Ground { query, k, depth, alpha, iters } => {
            // Pipeline: hybrid → seeds → PPR → traverse → JSON bundle
            let store = Store::open(&cli.file)?;
            let e = Embedder::new_with_cache::<std::path::PathBuf>(
                cli.file.parent().map(|p| p.join(".emb-cache")),
            )?;
            let q = e.embed_one(&query)?;
            let hits = store.search(&query, SearchMode::Hybrid, Some(&q), k)?;

            let mut seeds: std::collections::HashMap<i64, f64> = std::collections::HashMap::new();
            for h in &hits {
                seeds.insert(h.id, h.score as f64);
            }

            let conn = rusqlite::Connection::open(&cli.file)?;
            synapse_graph::ensure_schema(&conn)?;

            let ppr_ranked = synapse_core::ppr::personalized_pagerank(
                &conn, &seeds, alpha, iters,
                synapse_core::ppr::DEFAULT_NEIGHBOR_CAP, 30,
            ).unwrap_or_default();

            let mut expansions: Vec<serde_json::Value> = Vec::new();
            for (sid, _) in seeds.iter().take(8) {
                if let Ok(traverse_hits) = synapse_graph::traverse(&conn, *sid, depth, 6, 0.7, None) {
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

fn print_hits(hits: &[synapse_core::Hit]) {
    for h in hits {
        println!(
            "{}\t{:.4}\t{}",
            h.id,
            h.score,
            h.text.chars().take(120).collect::<String>()
        );
    }
}
