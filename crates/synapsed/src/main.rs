//! synapsed — persistent daemon. Unix socket + length-prefixed msgpack.

mod metrics;
mod proto;

use anyhow::{Context, Result};
use clap::Parser;
use proto::{PutReq, Request, Response};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use synapse_core::{embed::Embedder, snap, PutRequest, SearchMode, Store};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

#[derive(Parser)]
#[command(name = "synapsed", version, about = "Synapse daemon")]
struct Cli {
    #[arg(short = 'f', long, default_value = ".synapse/brain.db")]
    file: PathBuf,
    #[arg(short = 's', long, default_value = "/tmp/synapse.sock")]
    sock: PathBuf,
    /// Skip loading the embedding model at startup (lazy init on first use)
    #[arg(long, default_value_t = false)]
    lazy_embed: bool,
    /// Persistent embedding cache path (redb). Default: alongside db as .emb-cache.
    #[arg(long)]
    emb_cache: Option<PathBuf>,
    /// Restrict `Snap { out }` to this directory. Default: db parent dir.
    #[arg(long)]
    snap_dir: Option<PathBuf>,
    /// Max bytes accepted per `Put.text`. Default: 16 MiB.
    #[arg(long, default_value_t = 16 * 1024 * 1024)]
    max_put_bytes: usize,
    /// Prometheus metrics endpoint. Default: 127.0.0.1:9090. Env: SYNAPSE_METRICS_ADDR.
    #[arg(long, env = "SYNAPSE_METRICS_ADDR", default_value = "127.0.0.1:9090")]
    metrics_addr: SocketAddr,
}

struct State {
    store: Mutex<Store>,
    embedder: Mutex<Option<Embedder>>,
    db_path: PathBuf,
    cache_path: PathBuf,
    snap_dir: PathBuf,
    max_put_bytes: usize,
}

impl State {
    async fn ensure_embedder(&self) -> Result<()> {
        let mut g = self.embedder.lock().await;
        if g.is_none() {
            info!(
                "loading embedder (BGE-small-en-v1.5) cache={}…",
                self.cache_path.display()
            );
            let t0 = std::time::Instant::now();
            *g = Some(Embedder::new_with_cache(Some(&self.cache_path)).context("embedder init")?);
            info!("embedder ready in {:?}", t0.elapsed());
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "synapsed=info".into()),
        )
        .init();
    let cli = Cli::parse();
    if let Some(p) = cli.file.parent() {
        std::fs::create_dir_all(p).ok();
    }

    // Install Prometheus metrics recorder and spawn HTTP server
    let metrics_handle = metrics::MetricsHandle::install()?;
    let metrics_addr = cli.metrics_addr;
    tokio::spawn(metrics::serve(metrics_handle.handle.clone(), metrics_addr));

    let store = Store::open(&cli.file).context("open store")?;
    let cache_path = cli.emb_cache.clone().unwrap_or_else(|| {
        let mut p = cli.file.clone();
        let name = p
            .file_name()
            .map(|n| format!(".{}.emb-cache", n.to_string_lossy()))
            .unwrap_or_else(|| ".emb-cache".into());
        p.set_file_name(name);
        p
    });
    let embedder = if cli.lazy_embed {
        None
    } else {
        info!("warming embedder…");
        Some(Embedder::new_with_cache(Some(&cache_path)).context("embedder init")?)
    };
    let snap_dir = cli.snap_dir.clone().unwrap_or_else(|| {
        cli.file
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."))
    });
    std::fs::create_dir_all(&snap_dir).ok();
    let state = Arc::new(State {
        store: Mutex::new(store),
        embedder: Mutex::new(embedder),
        db_path: cli.file.clone(),
        cache_path,
        snap_dir,
        max_put_bytes: cli.max_put_bytes,
    });

    let _ = std::fs::remove_file(&cli.sock);
    let listener =
        UnixListener::bind(&cli.sock).with_context(|| format!("bind {}", cli.sock.display()))?;
    info!(
        "listening on {} (db={})",
        cli.sock.display(),
        cli.file.display()
    );

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(p) => p,
            Err(e) => {
                error!("accept: {e}");
                continue;
            }
        };
        let s = state.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_conn(stream, s).await {
                warn!("conn: {e}");
            }
        });
    }
}

async fn handle_conn(mut stream: UnixStream, state: Arc<State>) -> Result<()> {
    loop {
        let mut lenbuf = [0u8; 4];
        if stream.read_exact(&mut lenbuf).await.is_err() {
            return Ok(());
        }
        let len = u32::from_le_bytes(lenbuf) as usize;
        if len == 0 || len > 256 * 1024 * 1024 {
            return Err(anyhow::anyhow!("bad frame len {len}"));
        }
        let mut buf = vec![0u8; len];
        stream.read_exact(&mut buf).await?;
        let req: Request = rmp_serde::from_slice(&buf).context("decode request")?;
        let resp = dispatch(&state, req).await;
        let encoded = rmp_serde::to_vec_named(&resp)?;
        stream
            .write_all(&(encoded.len() as u32).to_le_bytes())
            .await?;
        stream.write_all(&encoded).await?;
        stream.flush().await?;
    }
}

async fn dispatch(state: &State, req: Request) -> Response {
    match req {
        Request::Ping => Response::Pong,
        Request::Put(p) => {
            let t0 = Instant::now();
            let result = put_one(state, p).await;
            metrics::record_put(t0.elapsed());
            match result {
                Ok(id) => Response::Id(id),
                Err(e) => Response::Err(e.to_string()),
            }
        }
        Request::PutBatch(batch) => {
            let t0 = Instant::now();
            let result = put_batch(state, batch).await;
            metrics::record_put(t0.elapsed());
            match result {
                Ok(ids) => Response::Ids(ids),
                Err(e) => Response::Err(e.to_string()),
            }
        }
        Request::Search {
            mode,
            q,
            limit,
            embed_query,
        } => {
            let mode_str = match mode {
                SearchMode::Lex => "lex",
                SearchMode::Vec => "vec",
                SearchMode::Hybrid => "hybrid",
            };
            let t0 = Instant::now();
            let result = search(state, mode, &q, limit, embed_query).await;
            metrics::record_query(mode_str, t0.elapsed());
            match result {
                Ok(hits) => Response::Hits(hits),
                Err(e) => Response::Err(e.to_string()),
            }
        }
        Request::Stats => match state.store.lock().await.stats() {
            Ok(s) => {
                metrics::set_doc_count(s.docs);
                Response::Stats {
                    docs: s.docs,
                    vecs: s.vecs,
                }
            }
            Err(e) => Response::Err(e.to_string()),
        },
        Request::Snap { out, level } => {
            let resolved = match sanitize_snap_path(&state.snap_dir, &out) {
                Ok(p) => p,
                Err(e) => return Response::Err(e.to_string()),
            };
            match snap::export(&state.db_path, &resolved, level) {
                Ok(()) => Response::Ok,
                Err(e) => Response::Err(e.to_string()),
            }
        }
        Request::Shutdown => {
            info!("shutdown requested");
            std::process::exit(0);
        }
        Request::Merge {
            id,
            state: crdt_state,
        } => match state.store.lock().await.merge_crdt(id, &crdt_state) {
            Ok(()) => Response::Ok,
            Err(e) => Response::Err(e.to_string()),
        },
        Request::Timeline { limit, offset } => {
            match state.store.lock().await.timeline(limit, offset) {
                Ok(docs) => Response::Docs(docs),
                Err(e) => Response::Err(e.to_string()),
            }
        }
        Request::Verify { id, vk } => {
            let arr_result: std::result::Result<[u8; 32], _> = vk.try_into();
            match arr_result {
                Err(_) => Response::Err("vk must be 32 bytes".into()),
                Ok(arr) => match ed25519_dalek::VerifyingKey::from_bytes(&arr) {
                    Err(e) => Response::Err(e.to_string()),
                    Ok(verifying_key) => {
                        match state.store.lock().await.verify(id, &verifying_key) {
                            Ok(()) => Response::Ok,
                            Err(e) => Response::Err(e.to_string()),
                        }
                    }
                },
            }
        }
    }
}

fn sanitize_snap_path(base: &std::path::Path, out: &str) -> Result<PathBuf> {
    let p = PathBuf::from(out);
    let absolute = if p.is_absolute() { p } else { base.join(p) };
    // Canonicalize what we can; canonicalize won't work if file doesn't exist yet, so canonicalize parent.
    let parent = absolute
        .parent()
        .ok_or_else(|| anyhow::anyhow!("no parent"))?;
    let canon_parent = parent
        .canonicalize()
        .unwrap_or_else(|_| parent.to_path_buf());
    let canon_base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    if !canon_parent.starts_with(&canon_base) {
        anyhow::bail!("snap path outside --snap-dir ({})", canon_base.display());
    }
    let fname = absolute
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("no filename"))?;
    Ok(canon_parent.join(fname))
}

async fn put_one(state: &State, p: PutReq) -> Result<i64> {
    if p.text.len() > state.max_put_bytes {
        anyhow::bail!("text too large: {} > {}", p.text.len(), state.max_put_bytes);
    }
    let embedding = if p.embed {
        state.ensure_embedder().await?;
        let g = state.embedder.lock().await;
        let e = g.as_ref().expect("embedder present");
        Some(e.embed_one(&p.text)?)
    } else {
        None
    };
    let mut req: PutRequest = p.into();
    req.embedding = embedding;
    let mut store = state.store.lock().await;
    Ok(store.put(&req)?)
}

async fn put_batch(state: &State, batch: Vec<PutReq>) -> Result<Vec<i64>> {
    for r in &batch {
        if r.text.len() > state.max_put_bytes {
            anyhow::bail!("text too large: {} > {}", r.text.len(), state.max_put_bytes);
        }
    }
    let any_embed = batch.iter().any(|r| r.embed);
    let embeddings = if any_embed {
        state.ensure_embedder().await?;
        let texts: Vec<String> = batch.iter().map(|r| r.text.clone()).collect();
        let g = state.embedder.lock().await;
        let e = g.as_ref().expect("embedder present");
        Some(e.embed_batch(&texts)?)
    } else {
        None
    };
    let reqs: Vec<PutRequest> = batch
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            let emb = if p.embed {
                embeddings.as_ref().map(|v| v[i].clone())
            } else {
                None
            };
            let mut r: PutRequest = p.into();
            r.embedding = emb;
            r
        })
        .collect();
    let mut store = state.store.lock().await;
    Ok(store.put_batch(&reqs)?)
}

async fn search(
    state: &State,
    mode: SearchMode,
    q: &str,
    limit: usize,
    embed_query: bool,
) -> Result<Vec<synapse_core::Hit>> {
    let emb = if embed_query {
        state.ensure_embedder().await?;
        let g = state.embedder.lock().await;
        let e = g.as_ref().expect("embedder present");
        Some(e.embed_one(q)?)
    } else {
        None
    };
    let t0 = std::time::Instant::now();
    let store = state.store.lock().await;
    let hits = store.search(q, mode, emb.as_deref(), limit)?;
    let latency_us = t0.elapsed().as_micros() as u64;
    let hit_count = hits.len();
    let top_score = hits.first().map(|h| h.score as f64).unwrap_or(0.0);
    if let Err(e) = store.log_query(q, mode, latency_us, hit_count, top_score) {
        warn!("query_log insert failed (non-fatal): {e}");
    }
    Ok(hits)
}
