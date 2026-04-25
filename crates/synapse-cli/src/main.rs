use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
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
        Cmd::Hybrid { query, limit } => {
            let store = Store::open(&cli.file)?;
            let e = Embedder::new_with_cache::<std::path::PathBuf>(
                cli.file.parent().map(|p| p.join(".emb-cache")),
            )?;
            let q = e.embed_one(&query)?;
            let hits = store.search(&query, SearchMode::Hybrid, Some(&q), limit)?;
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
