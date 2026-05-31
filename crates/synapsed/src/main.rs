//! synapsed — persistent daemon. Unix socket + length-prefixed msgpack.

// mimalloc — 5-15% faster than system malloc for many small allocs (msgpack, vec ops).
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod livequery;
mod metrics;
mod proto;

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use proto::{PutReq, Request, Response};
use rusqlite::params;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex as PlMutex;
use synapse_core::db::rrf_merge_neon;
use synapse_core::turbo::ndarray_search::NdArraySearch;
use synapse_core::{Hit, PutRequest, SearchMode, Store, embedder_trait::TextEmbedder, snap};
use synapse_rerank::{IdentityReranker, Reranker, build_reranker_from_env};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;
use tokio::time::timeout;
use tracing::{error, info, warn};

/// Idle timeout between requests on a kept-alive socket. Prevents fd leaks
/// from clients that connect but never send (or never disconnect cleanly).
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const SEARCH_SNIPPET_CHARS: usize = 512;
const QUERY_CACHE_MAX: usize = 4096;
const EMBED_CACHE_MAX: usize = 4096;
const DEFAULT_STATS_TTL_MS: u64 = 30_000;

type DynEmbedder = Box<dyn TextEmbedder>;
type QueryCacheMap = HashMap<String, Vec<Hit>>;
type StatsSnapshot = (Instant, i64, i64, i64);
type EmbedCacheMap = HashMap<String, Vec<f32>>;
type SqlRows = Vec<Vec<serde_json::Value>>;
type SqlResultRows = (Vec<String>, SqlRows);
type DocProjection = (Option<String>, Option<String>, String);
type DocProjectionMap = HashMap<i64, DocProjection>;

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
    /// Path to license JWT file (Pro+ tier). Default: ~/.config/synapse/license.jwt
    #[arg(long)]
    license_jwt: Option<PathBuf>,
    /// Ed25519 public key hex (32 bytes) used to verify the license JWT.
    #[arg(
        long,
        default_value = "0000000000000000000000000000000000000000000000000000000000000000"
    )]
    license_pubkey: String,
    /// Prometheus metrics endpoint. Default: 127.0.0.1:9090. Env: SYNAPSE_METRICS_ADDR.
    #[arg(long, env = "SYNAPSE_METRICS_ADDR", default_value = "127.0.0.1:9090")]
    metrics_addr: SocketAddr,
    /// LiveQuery WebSocket endpoint. Default: 127.0.0.1:9091. Env: SYNAPSE_LIVE_ADDR.
    #[arg(long, env = "SYNAPSE_LIVE_ADDR", default_value = "127.0.0.1:9091")]
    live_addr: SocketAddr,
    /// Path to ONNX cross-encoder model for reranking (requires --features onnx).
    /// Default: ~/.synapse/models/ms-marco-MiniLM-L-6-v2.onnx (auto-download via fastembed).
    /// If not set and onnx feature is active, BGE-reranker-v2-m3 is auto-downloaded on first use.
    #[arg(long)]
    rerank_model: Option<PathBuf>,
}

struct State {
    store: PlMutex<Store>,
    /// Hot ANN index lifted out of store mutex — lock-free reads via RwLock.
    /// Writes (put/put_batch) update both store and this index under store mutex,
    /// then also update this index under write lock (rare, no perf concern).
    ndarray_idx: Arc<parking_lot::RwLock<Option<NdArraySearch>>>,
    embedder: Mutex<Option<DynEmbedder>>,
    embedder_init: Mutex<()>,
    reranker: Box<dyn Reranker>,
    db_path: PathBuf,
    cache_path: PathBuf,
    snap_dir: PathBuf,
    max_put_bytes: usize,
    /// Read-only PRAGMA-tuned connection for Sql op. Avoid per-call open/PRAGMA overhead.
    sql_conn: PlMutex<Option<rusqlite::Connection>>,
    /// Hot read-through cache for repeated user/agent queries. Cleared on writes.
    query_cache: PlMutex<QueryCacheMap>,
    /// TTL cache for stats; COUNT over docs_vec is measurable at 178k+ vecs.
    /// Daemon-owned writes update this cache exactly, so hot agent status checks
    /// stay O(1) while external DB writers are still picked up after the TTL.
    stats_cache: PlMutex<Option<StatsSnapshot>>,
    stats_ttl: Duration,
    /// MLX query embeddings are fast after warmup but still milliseconds; cache hot prompts.
    embed_cache: PlMutex<EmbedCacheMap>,
    /// LiveQuery broadcast broker (P2.2). Emits on Put/PutBatch/Merge.
    live_broker: livequery::LiveBroker,
}

impl State {
    async fn ensure_embedder(&self) -> Result<()> {
        {
            let g = self.embedder.lock().await;
            if g.is_some() {
                return Ok(());
            }
        }
        let _init = self.embedder_init.lock().await;
        {
            let g = self.embedder.lock().await;
            if g.is_some() {
                return Ok(());
            }
        }

        info!(
            "loading embedder (BGE-small-en-v1.5) cache={}…",
            self.cache_path.display()
        );
        let t0 = std::time::Instant::now();
        let cache_path = self.cache_path.clone();
        let embedder = tokio::task::spawn_blocking(move || {
            synapse_core::embed::pick_embedder_with_cache(Some(&cache_path))
        })
        .await
        .map_err(|e| anyhow!("embedder init task failed: {e}"))?;
        let mut g = self.embedder.lock().await;
        *g = Some(embedder);
        info!("embedder ready in {:?}", t0.elapsed());
        Ok(())
    }
}

fn build_reranker(_model_path: &Option<PathBuf>) -> Box<dyn Reranker> {
    // Check SYNAPSE_RERANKER env first (supports identity / lightgbm:PATH / onnx).
    if std::env::var("SYNAPSE_RERANKER").is_ok() {
        return build_reranker_from_env();
    }
    // Legacy: feature-flag driven fallback (no env set).
    #[cfg(feature = "onnx")]
    {
        info!("onnx feature active — loading OnnxCrossEncoder (BGE-reranker-v2-m3)…");
        match synapse_rerank::onnx::OnnxCrossEncoder::new() {
            Ok(enc) => {
                info!("reranker ready (BGE-reranker-v2-m3)");
                return Box::new(enc);
            }
            Err(e) => {
                warn!("OnnxCrossEncoder init failed ({e}) — falling back to IdentityReranker");
            }
        }
    }
    #[cfg(not(feature = "onnx"))]
    info!("reranker: IdentityReranker (build without --features onnx for real cross-encoder)");
    Box::new(IdentityReranker)
}

fn env_duration_ms(name: &str, default_ms: u64) -> Duration {
    let ms = std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default_ms);
    Duration::from_millis(ms)
}

fn open_store(
    file: &PathBuf,
    license_jwt_path: &Option<PathBuf>,
    pubkey_hex: &str,
) -> Result<Store> {
    #[cfg(feature = "licensed")]
    {
        let jwt_path = license_jwt_path.clone().unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config/synapse/license.jwt")
        });
        if jwt_path.exists() {
            let jwt = match std::fs::read_to_string(&jwt_path) {
                Ok(s) => s.trim().to_owned(),
                Err(e) => {
                    warn!(
                        "failed to read license file {}: {e} — falling back to free tier",
                        jwt_path.display()
                    );
                    return Store::open(file).context("open store (free tier)");
                }
            };
            let pubkey_bytes = match hex::decode(pubkey_hex) {
                Ok(b) if b.len() == 32 => b,
                _ => {
                    warn!(
                        "invalid --license-pubkey (must be 32-byte hex) — falling back to free tier"
                    );
                    return Store::open(file).context("open store (free tier)");
                }
            };
            match synapse_license::verify_license(&jwt, &pubkey_bytes) {
                Ok(license) => {
                    info!(
                        "license verified: customer={} tier={}",
                        license.customer_id, license.tier
                    );
                    let hw_fp = synapse_license::current_hw_fingerprint();
                    // Use the raw JWT bytes as the "signature" material for brain_key derivation.
                    let brain_key = synapse_core::db::derive_brain_key(jwt.as_bytes(), &hw_fp);
                    info!("brain_key derived — opening encrypted store");
                    return Store::open_with_brain_key(file, &brain_key)
                        .context("open encrypted store");
                }
                Err(e) => {
                    warn!("license invalid ({e}) — falling back to free tier (unencrypted)");
                    return Store::open(file).context("open store (free tier)");
                }
            }
        } else {
            info!(
                "no license file found at {} — running free tier",
                jwt_path.display()
            );
        }
    }
    #[cfg(not(feature = "licensed"))]
    let _ = (license_jwt_path, pubkey_hex);
    Store::open(file).context("open store")
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

    let store = open_store(&cli.file, &cli.license_jwt, &cli.license_pubkey)?;
    // Keep startup default-alive: bind the socket first, then warm the turbo
    // matrix in the background. Fast reads use the top-level index once ready.
    let ndarray_idx = Arc::new(parking_lot::RwLock::new(None));
    let cache_path = cli.emb_cache.clone().unwrap_or_else(|| {
        let mut p = cli.file.clone();
        let name = p
            .file_name()
            .map(|n| format!(".{}.emb-cache", n.to_string_lossy()))
            .unwrap_or_else(|| ".emb-cache".into());
        p.set_file_name(name);
        p
    });
    let warm_embedder = !cli.lazy_embed;
    let embedder: Option<Box<dyn TextEmbedder>> = None;
    let snap_dir = cli.snap_dir.clone().unwrap_or_else(|| {
        cli.file
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."))
    });
    std::fs::create_dir_all(&snap_dir).ok();
    let reranker: Box<dyn Reranker> = build_reranker(&cli.rerank_model);
    let stats_ttl = env_duration_ms("SYNAPSE_STATS_TTL_MS", DEFAULT_STATS_TTL_MS);
    // Pre-open PRAGMA-tuned read-only Sql conn for analytics ops.
    let sql_conn = {
        use rusqlite::{Connection, OpenFlags};
        match Connection::open_with_flags(
            &cli.file,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        ) {
            Ok(c) => {
                let _ = c.pragma_update(None, "mmap_size", 1_073_741_824_i64);
                let _ = c.pragma_update(None, "cache_size", -262_144_i64);
                let _ = c.pragma_update(None, "temp_store", 2_i64);
                Some(c)
            }
            Err(e) => {
                tracing::warn!("sql_conn pre-open failed: {e}");
                None
            }
        }
    };
    let state = Arc::new(State {
        store: PlMutex::new(store),
        ndarray_idx,
        embedder: Mutex::new(embedder),
        embedder_init: Mutex::new(()),
        reranker,
        db_path: cli.file.clone(),
        cache_path,
        snap_dir,
        max_put_bytes: cli.max_put_bytes,
        sql_conn: PlMutex::new(sql_conn),
        query_cache: PlMutex::new(HashMap::new()),
        stats_cache: PlMutex::new(None),
        stats_ttl,
        embed_cache: PlMutex::new(HashMap::new()),
        live_broker: livequery::LiveBroker::new(256),
    });

    // Spawn LiveQuery WebSocket server (P2.2).
    let live_broker_clone = state.live_broker.clone();
    let live_addr = cli.live_addr;
    tokio::spawn(async move {
        livequery::serve(live_broker_clone, live_addr).await;
    });

    let _ = std::fs::remove_file(&cli.sock);
    let listener =
        UnixListener::bind(&cli.sock).with_context(|| format!("bind {}", cli.sock.display()))?;
    info!(
        "listening on {} (db={})",
        cli.sock.display(),
        cli.file.display()
    );

    if warm_embedder {
        let warm_state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = warm_state.ensure_embedder().await {
                warn!("background embedder warmup failed: {e}");
            }
        });
    }

    spawn_turbo_warm(state.clone(), cli.file.clone());

    // SIGTERM/SIGINT handler — persist ANN sidecar before exit.
    // Saves 5min HNSW rebuild on next start.
    let sig_state = state.clone();
    tokio::spawn(async move {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm = signal(SignalKind::terminate()).expect("install SIGTERM");
        let mut sigint = signal(SignalKind::interrupt()).expect("install SIGINT");
        tokio::select! {
            _ = sigterm.recv() => info!("SIGTERM received"),
            _ = sigint.recv() => info!("SIGINT received"),
        }
        info!("persisting ANN sidecar before exit…");
        #[cfg(feature = "ann-usearch")]
        {
            let store = sig_state.store.lock();
            if let Err(e) = store.flush_ann() {
                tracing::warn!("flush_ann on signal failed: {e}");
            } else {
                info!("ANN sidecar persisted");
            }
        }
        std::process::exit(0);
    });

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

fn spawn_turbo_warm(state: Arc<State>, db_path: PathBuf) {
    tokio::spawn(async move {
        let build = tokio::task::spawn_blocking(move || -> Result<NdArraySearch> {
            use rusqlite::{Connection, OpenFlags};
            let conn = Connection::open_with_flags(
                &db_path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
            )?;
            let _ = conn.pragma_update(None, "mmap_size", 1_073_741_824_i64);
            let _ = conn.pragma_update(None, "cache_size", -262_144_i64);
            let _ = conn.pragma_update(None, "temp_store", 2_i64);
            NdArraySearch::from_connection(&conn).map_err(Into::into)
        })
        .await;
        match build {
            Ok(Ok(idx)) => {
                let len = idx.len();
                *state.ndarray_idx.write() = Some(idx);
                info!("turbo ndarray_search warmed in background: {len} vectors");
            }
            Ok(Err(e)) => warn!("background turbo warmup failed: {e}"),
            Err(e) => warn!("background turbo warmup task failed: {e}"),
        }
    });
}

async fn handle_conn(mut stream: UnixStream, state: Arc<State>) -> Result<()> {
    let api_key = std::env::var("SYNAPSE_API_KEY").ok();
    let auth_required = api_key.is_some();
    let mut authed = !auth_required; // session-scoped auth state
    loop {
        let mut lenbuf = [0u8; 4];
        match timeout(IDLE_TIMEOUT, stream.read_exact(&mut lenbuf)).await {
            Ok(Ok(_)) => {}
            Ok(Err(_)) => return Ok(()),
            Err(_) => return Ok(()),
        }
        let len = u32::from_le_bytes(lenbuf) as usize;
        if len == 0 || len > 256 * 1024 * 1024 {
            return Err(anyhow::anyhow!("bad frame len {len}"));
        }
        let mut buf = vec![0u8; len];
        stream.read_exact(&mut buf).await?;
        let req: Request = rmp_serde::from_slice(&buf).context("decode request")?;
        // P2.4 auth gate: require Auth before any op except Ping/Stats/Auth itself.
        let resp = if !authed {
            match &req {
                Request::Ping | Request::Stats | Request::Auth { .. } => {}
                _ => {
                    let r = Response::Err("auth required: send Auth{token}".into());
                    let encoded = rmp_serde::to_vec_named(&r)?;
                    stream
                        .write_all(&(encoded.len() as u32).to_le_bytes())
                        .await?;
                    stream.write_all(&encoded).await?;
                    stream.flush().await?;
                    continue;
                }
            }
            // Process as normal — may include Auth which sets authed=true below.
            let r = dispatch(&state, req).await;
            // If just authed successfully, flip flag.
            if let Response::Ok = r {
                authed = true;
            }
            r
        } else {
            dispatch(&state, req).await
        };
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
            let title = p.title.clone();
            let uri = p.uri.clone();
            let result = put_one(state, p).await;
            metrics::record_put(t0.elapsed());
            match result {
                Ok(id) => {
                    // P2.2 LiveQuery emit
                    state.live_broker.emit(livequery::LiveEvent {
                        op: "Put".into(),
                        id,
                        title,
                        uri,
                        ts: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs() as i64)
                            .unwrap_or(0),
                    });
                    Response::Id(id)
                }
                Err(e) => Response::Err(e.to_string()),
            }
        }
        Request::PutBatch(batch) => {
            let t0 = Instant::now();
            let titles: Vec<_> = batch
                .iter()
                .map(|p| (p.title.clone(), p.uri.clone()))
                .collect();
            let result = put_batch(state, batch).await;
            metrics::record_put(t0.elapsed());
            match result {
                Ok(ids) => {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    for (id, (title, uri)) in ids.iter().zip(titles.iter()) {
                        state.live_broker.emit(livequery::LiveEvent {
                            op: "PutBatch".into(),
                            id: *id,
                            title: title.clone(),
                            uri: uri.clone(),
                            ts: now,
                        });
                    }
                    Response::Ids(ids)
                }
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
        Request::SearchScoped {
            mode,
            q,
            limit,
            embed_query,
            scope_key,
            scope_value,
            candidate_limit,
        } => {
            let mode_str = match mode {
                SearchMode::Lex => "lex_scoped",
                SearchMode::Vec => "vec_scoped",
                SearchMode::Hybrid => "hybrid_scoped",
            };
            let t0 = Instant::now();
            let result = search_scoped(
                state,
                mode,
                &q,
                limit,
                embed_query,
                scope_key.as_deref().unwrap_or("scope"),
                &scope_value,
                candidate_limit,
            )
            .await;
            metrics::record_query(mode_str, t0.elapsed());
            match result {
                Ok(hits) => Response::Hits(hits),
                Err(e) => Response::Err(e.to_string()),
            }
        }
        Request::Stats => {
            if let Some((at, docs, vecs, _max_id)) = *state.stats_cache.lock()
                && at.elapsed() < state.stats_ttl
            {
                return Response::Stats { docs, vecs };
            }
            match state.store.lock().stats() {
                Ok(s) => {
                    metrics::set_doc_count(s.docs);
                    let max_id = current_max_doc_id(state).unwrap_or(s.docs);
                    *state.stats_cache.lock() = Some((Instant::now(), s.docs, s.vecs, max_id));
                    Response::Stats {
                        docs: s.docs,
                        vecs: s.vecs,
                    }
                }
                Err(e) => Response::Err(e.to_string()),
            }
        }
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
            info!("shutdown requested — persisting ANN sidecar");
            #[cfg(feature = "ann-usearch")]
            {
                let store = state.store.lock();
                let has_ann = store.has_ann();
                let ann_len = store.ann_len();
                info!("has_ann={} ann_len={}", has_ann, ann_len);
                match store.flush_ann() {
                    Ok(()) => info!("flush_ann OK"),
                    Err(e) => tracing::warn!("flush_ann on shutdown failed: {e}"),
                }
            }
            info!("exit 0");
            std::process::exit(0);
        }
        Request::Merge {
            id,
            state: crdt_state,
        } => match state.store.lock().merge_crdt(id, &crdt_state) {
            Ok(()) => {
                clear_query_cache(state);
                Response::Ok
            }
            Err(e) => Response::Err(e.to_string()),
        },
        Request::Delete { id } => match state.store.lock().delete(id) {
            Ok(_) => {
                clear_query_cache(state);
                Response::Ok
            }
            Err(e) => Response::Err(e.to_string()),
        },
        Request::Timeline { limit, offset } => match state.store.lock().timeline(limit, offset) {
            Ok(docs) => Response::Docs(docs),
            Err(e) => Response::Err(e.to_string()),
        },
        Request::Verify { id, vk } => {
            let arr_result: std::result::Result<[u8; 32], _> = vk.try_into();
            match arr_result {
                Err(_) => Response::Err("vk must be 32 bytes".into()),
                Ok(arr) => match ed25519_dalek::VerifyingKey::from_bytes(&arr) {
                    Err(e) => Response::Err(e.to_string()),
                    Ok(verifying_key) => match state.store.lock().verify(id, &verifying_key) {
                        Ok(()) => Response::Ok,
                        Err(e) => Response::Err(e.to_string()),
                    },
                },
            }
        }
        Request::Embed { text, dim } => {
            match embed_one(state, &text).await {
                Ok(mut vec) => {
                    // Matryoshka truncation: if dim provided and < native, truncate + L2-renorm.
                    if let Some(d) = dim
                        && d < vec.len()
                        && d > 0
                    {
                        vec.truncate(d);
                        let n: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
                        if n > 1e-10 {
                            for x in vec.iter_mut() {
                                *x /= n;
                            }
                        }
                    }
                    Response::Embed { vec }
                }
                Err(e) => Response::Err(e.to_string()),
            }
        }
        Request::SnapMerge {
            snapshot_path,
            out_path,
            level,
        } => {
            let db_path = state.db_path.clone();
            let tmp =
                std::env::temp_dir().join(format!("synapse-snap-{}.brainpack", std::process::id()));
            match synapse_core::snap::export(&db_path, &tmp, level).and_then(|_| {
                synapse_core::snap::merge_packs(
                    &tmp,
                    std::path::Path::new(&snapshot_path),
                    std::path::Path::new(&out_path),
                    level,
                )
            }) {
                Ok(_) => {
                    let _ = std::fs::remove_file(&tmp);
                    Response::Ok
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp);
                    Response::Err(e.to_string())
                }
            }
        }
        Request::SearchVec { embedding, limit } => {
            // Hot path: ANN search on the lifted RwLock index (no Store mutex).
            // Multiple concurrent readers proceed in parallel on the 12-core M4 Max.
            let ann_pairs = {
                let guard = state.ndarray_idx.read();
                if let Some(ref idx) = *guard {
                    if !idx.is_empty() {
                        let binary_k = 4096usize.max(limit * 64).min(idx.len());
                        if binary_k < idx.len() {
                            Some(idx.search_cascade(&embedding, limit, binary_k))
                        } else {
                            Some(idx.search(&embedding, limit))
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            if let Some(pairs) = ann_pairs {
                // Hydrate id→snippet through the read-only SQL connection.
                // This avoids the Store mutex and avoids pulling multi-KB full texts
                // for search result previews.
                let result =
                    tokio::task::block_in_place(|| hydrate_pairs_from_state(state, &pairs));
                match result {
                    Ok(hits) => Response::Hits(hits),
                    Err(e) => Response::Err(e.to_string()),
                }
            } else {
                // Fallback: full store search (index cold or empty).
                let result = tokio::task::block_in_place(|| {
                    let store = state.store.lock();
                    store.search("", SearchMode::Vec, Some(&embedding), limit)
                });
                match result {
                    Ok(hits) => Response::Hits(hits),
                    Err(e) => Response::Err(e.to_string()),
                }
            }
        }
        Request::Rerank {
            query,
            candidates,
            top_k,
        } => match state.reranker.rerank(&query, candidates, top_k) {
            Ok(hits) => Response::Hits(hits),
            Err(e) => Response::Err(e.to_string()),
        },
        Request::BatchSearch { queries } => {
            // Sequential per-query, single roundtrip — saves N socket cycles.
            let mut all = Vec::with_capacity(queries.len());
            for item in queries {
                match search(state, item.mode, &item.q, item.limit, item.embed_query).await {
                    Ok(hits) => all.push(hits),
                    Err(_) => all.push(Vec::new()),
                }
            }
            Response::BatchHits(all)
        }
        Request::UseTenant { name } => {
            // P4.1 simple multi-tenant: validate name + ATTACH read-only.
            // Tenant DBs at ~/.synapse/tenants/{name}.db.
            if !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Response::Err("invalid tenant name (alnum/_/- only)".into());
            }
            let path = format!(
                "{}/.synapse/tenants/{}.db",
                std::env::var("HOME").unwrap_or_default(),
                name
            );
            if !std::path::Path::new(&path).exists() {
                return Response::Err(format!("tenant db not found: {path}"));
            }
            let result: std::result::Result<(), String> = tokio::task::block_in_place(|| {
                let mut g = state.sql_conn.lock();
                if let Some(conn) = g.as_mut() {
                    conn.execute("DETACH DATABASE tenant", []).ok();
                    conn.execute(
                        &format!("ATTACH DATABASE 'file:{}?mode=ro' AS tenant", path),
                        [],
                    )
                    .map_err(|e| e.to_string())?;
                }
                Ok(())
            });
            match result {
                Ok(()) => Response::Ok,
                Err(e) => Response::Err(e),
            }
        }
        Request::Auth { token } => {
            // Constant-time compare to thwart timing attacks (basic).
            match std::env::var("SYNAPSE_API_KEY") {
                Ok(k) if !k.is_empty() => {
                    if subtle_eq(&token, &k) {
                        Response::Ok
                    } else {
                        Response::Err("invalid token".into())
                    }
                }
                _ => Response::Ok, // no key configured — auth always passes
            }
        }
        Request::Transaction { ops } => {
            // Atomic batch via put_batch (single SQL transaction in store).
            let result = put_batch(state, ops).await;
            match result {
                Ok(ids) => Response::Ids(ids),
                Err(e) => Response::Err(e.to_string()),
            }
        }
        Request::Sql { query, params } => {
            // Read-only raw SQL via pooled PRAGMA-tuned conn (avoid per-call open + PRAGMA cost).
            let result: Result<SqlResultRows, String> = tokio::task::block_in_place(|| {
                use rusqlite::types::ValueRef;
                let guard = state.sql_conn.lock();
                let conn = guard
                    .as_ref()
                    .ok_or_else(|| "sql_conn unavailable".to_string())?;
                let mut stmt = conn.prepare_cached(&query).map_err(|e| e.to_string())?;
                let cols: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
                let n_cols = cols.len();
                let rusq_params: Vec<rusqlite::types::Value> = params
                    .iter()
                    .map(|v| match v {
                        serde_json::Value::Null => rusqlite::types::Value::Null,
                        serde_json::Value::Bool(b) => rusqlite::types::Value::Integer(*b as i64),
                        serde_json::Value::Number(n) => {
                            if let Some(i) = n.as_i64() {
                                rusqlite::types::Value::Integer(i)
                            } else if let Some(f) = n.as_f64() {
                                rusqlite::types::Value::Real(f)
                            } else {
                                rusqlite::types::Value::Null
                            }
                        }
                        serde_json::Value::String(s) => rusqlite::types::Value::Text(s.clone()),
                        _ => rusqlite::types::Value::Text(v.to_string()),
                    })
                    .collect();
                let param_refs: Vec<&dyn rusqlite::ToSql> = rusq_params
                    .iter()
                    .map(|v| v as &dyn rusqlite::ToSql)
                    .collect();
                let mut rows_out = Vec::new();
                let mut rows_iter = stmt
                    .query(rusqlite::params_from_iter(&param_refs))
                    .map_err(|e| e.to_string())?;
                while let Some(row) = rows_iter.next().map_err(|e| e.to_string())? {
                    let mut row_vals = Vec::with_capacity(n_cols);
                    for i in 0..n_cols {
                        let v = row.get_ref(i).map_err(|e| e.to_string())?;
                        row_vals.push(match v {
                            ValueRef::Null => serde_json::Value::Null,
                            ValueRef::Integer(i) => serde_json::Value::from(i),
                            ValueRef::Real(f) => serde_json::Value::from(f),
                            ValueRef::Text(t) => {
                                serde_json::Value::String(String::from_utf8_lossy(t).into_owned())
                            }
                            ValueRef::Blob(b) => {
                                serde_json::Value::String(format!("<blob:{}b>", b.len()))
                            }
                        });
                    }
                    rows_out.push(row_vals);
                }
                Ok((cols, rows_out))
            });
            match result {
                Ok((cols, rows)) => Response::Rows { cols, rows },
                Err(e) => Response::Err(e),
            }
        }
    }
}

/// Constant-time equality compare — prevents timing attacks on token check.
fn subtle_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut acc: u8 = 0;
    for (x, y) in a.as_bytes().iter().zip(b.as_bytes().iter()) {
        acc |= x ^ y;
    }
    acc == 0
}

async fn embed_one(state: &State, text: &str) -> Result<Vec<f32>> {
    state.ensure_embedder().await?;
    let g = state.embedder.lock().await;
    let e = g.as_ref().expect("embedder present after ensure");
    Ok(e.embed_one(text)?)
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
    let embedding = if let Some(v) = p.embedding.clone() {
        Some(v)
    } else if p.embed {
        state.ensure_embedder().await?;
        let g = state.embedder.lock().await;
        let e = g.as_ref().expect("embedder present");
        Some(e.embed_one(&p.text)?)
    } else {
        None
    };
    let mut req: PutRequest = p.into();
    req.embedding = embedding;
    let id = tokio::task::block_in_place(|| {
        let mut store = state.store.lock();
        store.put(&req)
    })?;
    adjust_stats_cache_after_put(state, id, req.embedding.is_some());
    if let Some(ref emb) = req.embedding {
        update_lifted_ndarray(state, id, emb);
    }
    clear_query_cache(state);
    Ok(id)
}

async fn put_batch(state: &State, batch: Vec<PutReq>) -> Result<Vec<i64>> {
    for r in &batch {
        if r.text.len() > state.max_put_bytes {
            anyhow::bail!("text too large: {} > {}", r.text.len(), state.max_put_bytes);
        }
    }
    let need_server_embed: Vec<bool> = batch
        .iter()
        .map(|r| r.embed && r.embedding.is_none())
        .collect();
    let any_embed = need_server_embed.iter().any(|b| *b);
    let server_embeddings = if any_embed {
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
            let emb = if let Some(v) = p.embedding.clone() {
                Some(v)
            } else if need_server_embed[i] {
                server_embeddings.as_ref().map(|v| v[i].clone())
            } else {
                None
            };
            let mut r: PutRequest = p.into();
            r.embedding = emb;
            r
        })
        .collect();
    let ids = tokio::task::block_in_place(|| {
        let mut store = state.store.lock();
        store.put_batch(&reqs)
    })?;
    adjust_stats_cache_after_batch(state, &ids, &reqs);
    for (id, req) in ids.iter().zip(reqs.iter()) {
        if let Some(ref emb) = req.embedding {
            update_lifted_ndarray(state, *id, emb);
        }
    }
    clear_query_cache(state);
    Ok(ids)
}

fn update_lifted_ndarray(state: &State, id: i64, embedding: &[f32]) {
    let mut guard = state.ndarray_idx.write();
    let Some(ref mut idx) = *guard else {
        return;
    };
    if let Err(e) = idx.add_row(id, embedding) {
        warn!("lifted ndarray add_row id {id} failed: {e}; invalidating hot index");
        *guard = None;
    }
}

async fn search(
    state: &State,
    mode: SearchMode,
    q: &str,
    limit: usize,
    embed_query: bool,
) -> Result<Vec<synapse_core::Hit>> {
    if let Some(hits) = get_cached_hits(state, mode, q, limit, embed_query) {
        return Ok(hits);
    }

    let emb = if embed_query {
        if let Some(v) = get_cached_embedding(state, q) {
            Some(v)
        } else {
            state.ensure_embedder().await?;
            let g = state.embedder.lock().await;
            let e = g.as_ref().expect("embedder present");
            let v = e.embed_one(q)?;
            put_cached_embedding(state, q, &v);
            Some(v)
        }
    } else {
        None
    };

    let fast_t0 = std::time::Instant::now();
    if let Some(hits) = tokio::task::block_in_place(|| {
        fast_search_from_state(state, mode, q, emb.as_deref(), limit)
    })? {
        let latency_us = fast_t0.elapsed().as_micros() as u64;
        let cnt = hits.len();
        let top = hits.first().map(|x| x.score).unwrap_or(0.0);
        let log_err = {
            let store = state.store.lock();
            store.log_query(q, mode, latency_us, cnt, top).err()
        };
        if let Some(e) = log_err {
            warn!("query_log insert failed (non-fatal): {e}");
        }
        put_cached_hits(state, mode, q, limit, embed_query, &hits);
        return Ok(hits);
    }

    let t0 = std::time::Instant::now();
    let q_owned = q.to_owned();
    let emb_owned = emb;
    let (hits, latency_us, hit_count, top_score, log_err) = tokio::task::block_in_place(|| {
        let store = state.store.lock();
        let h = store.search(&q_owned, mode, emb_owned.as_deref(), limit)?;
        let lat = t0.elapsed().as_micros() as u64;
        let cnt = h.len();
        let top = h.first().map(|x| x.score).unwrap_or(0.0);
        let lerr = store.log_query(&q_owned, mode, lat, cnt, top).err();
        Ok::<_, synapse_core::Error>((h, lat, cnt, top, lerr))
    })?;
    if let Some(e) = log_err {
        warn!("query_log insert failed (non-fatal): {e}");
    }
    let _ = (latency_us, hit_count, top_score);
    put_cached_hits(state, mode, q, limit, embed_query, &hits);
    Ok(hits)
}

#[allow(clippy::too_many_arguments)]
async fn search_scoped(
    state: &State,
    mode: SearchMode,
    q: &str,
    limit: usize,
    embed_query: bool,
    scope_key: &str,
    scope_value: &str,
    candidate_limit: Option<usize>,
) -> Result<Vec<synapse_core::Hit>> {
    let limit = limit.max(1);
    let fetch_k = candidate_limit
        .unwrap_or_else(|| limit.saturating_mul(10))
        .max(limit);
    let scope_key = sanitize_meta_key(scope_key)?;

    let mut candidates = tokio::task::block_in_place(|| {
        scoped_lex_from_state(state, q, &scope_key, scope_value, fetch_k)
    })?;

    if candidates.len() < limit && !matches!(mode, SearchMode::Lex) {
        let global = search(state, mode, q, fetch_k, embed_query).await?;
        let filtered = tokio::task::block_in_place(|| {
            filter_hits_by_meta(state, &global, &scope_key, scope_value)
        })?;
        candidates.extend(filtered);
    }

    Ok(rank_scoped_hits(q, candidates, limit))
}

fn mode_cache_tag(mode: SearchMode) -> &'static str {
    match mode {
        SearchMode::Lex => "lex",
        SearchMode::Vec => "vec",
        SearchMode::Hybrid => "hybrid",
    }
}

fn search_cache_key(mode: SearchMode, q: &str, limit: usize, embed_query: bool) -> String {
    format!(
        "{}:{}:{}:{}",
        mode_cache_tag(mode),
        limit.max(1),
        embed_query as u8,
        q
    )
}

fn sanitize_meta_key(key: &str) -> Result<String> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("scope_key must not be empty"));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(anyhow!(
            "scope_key may only contain ASCII letters, digits and underscore"
        ));
    }
    Ok(trimmed.to_string())
}

fn scoped_lex_from_state(
    state: &State,
    q: &str,
    scope_key: &str,
    scope_value: &str,
    limit: usize,
) -> Result<Vec<Hit>> {
    let guard = state.sql_conn.lock();
    let conn = guard
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("sql_conn unavailable"))?;
    let json_path = format!("$.{scope_key}");
    let use_bm25 = should_rank_lex_with_bm25(q);
    if let Some(fts_query) = fts5_loose_match_query(q) {
        let sql = if use_bm25 {
            format!(
                "SELECT d.id,d.uri,d.title,substr(d.text,1,{SEARCH_SNIPPET_CHARS}),bm25(docs_fts) as score
                 FROM docs_fts JOIN docs d ON d.id = docs_fts.rowid
                 WHERE docs_fts MATCH ?1
                   AND d.meta IS NOT NULL AND json_valid(d.meta)
                   AND json_extract(d.meta, ?2) = ?3
                 ORDER BY score LIMIT ?4"
            )
        } else {
            format!(
                "SELECT d.id,d.uri,d.title,substr(d.text,1,{SEARCH_SNIPPET_CHARS}),0.0 as score
                 FROM docs_fts JOIN docs d ON d.id = docs_fts.rowid
                 WHERE docs_fts MATCH ?1
                   AND d.meta IS NOT NULL AND json_valid(d.meta)
                   AND json_extract(d.meta, ?2) = ?3
                 LIMIT ?4"
            )
        };
        let mut stmt = conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(
            params![fts_query, json_path, scope_value, limit as i64],
            |r| {
                let raw_score = r.get::<_, f64>(4)?;
                Ok(Hit {
                    id: r.get(0)?,
                    uri: r.get(1)?,
                    title: r.get(2)?,
                    text: r.get(3)?,
                    score: if use_bm25 { -raw_score } else { raw_score },
                    meta: None,
                    ts: None,
                })
            },
        )?;
        return Ok(rows.collect::<rusqlite::Result<_>>()?);
    }

    let sql = format!(
        "SELECT id,uri,title,substr(text,1,{SEARCH_SNIPPET_CHARS}),0.0 as score
         FROM docs
         WHERE meta IS NOT NULL AND json_valid(meta)
           AND json_extract(meta, ?1) = ?2
         ORDER BY ts DESC, id DESC LIMIT ?3"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt.query_map(params![json_path, scope_value, limit as i64], |r| {
        Ok(Hit {
            id: r.get(0)?,
            uri: r.get(1)?,
            title: r.get(2)?,
            text: r.get(3)?,
            score: r.get(4)?,
            meta: None,
            ts: None,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn filter_hits_by_meta(
    state: &State,
    hits: &[Hit],
    scope_key: &str,
    scope_value: &str,
) -> Result<Vec<Hit>> {
    if hits.is_empty() {
        return Ok(Vec::new());
    }
    let guard = state.sql_conn.lock();
    let conn = guard
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("sql_conn unavailable"))?;
    let json_path = format!("$.{scope_key}");
    let ids: Vec<i64> = hits.iter().map(|h| h.id).collect();
    let placeholders = (0..ids.len())
        .map(|i| format!("?{}", i + 3))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT id FROM docs
         WHERE meta IS NOT NULL AND json_valid(meta)
           AND json_extract(meta, ?1) = ?2
           AND id IN ({placeholders})"
    );
    let mut params_vec: Vec<&dyn rusqlite::ToSql> = vec![&json_path, &scope_value];
    for id in &ids {
        params_vec.push(id);
    }
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt.query_map(params_vec.as_slice(), |r| r.get::<_, i64>(0))?;
    let scoped_ids: std::collections::HashSet<i64> = rows.collect::<rusqlite::Result<_>>()?;
    Ok(hits
        .iter()
        .filter(|hit| scoped_ids.contains(&hit.id))
        .cloned()
        .collect())
}

fn rank_scoped_hits(q: &str, hits: Vec<Hit>, limit: usize) -> Vec<Hit> {
    let terms = query_terms(q);
    let mut seen = std::collections::HashSet::new();
    let mut unique = Vec::with_capacity(hits.len());
    for hit in hits {
        if seen.insert(hit.id) {
            unique.push(hit);
        }
    }
    unique.sort_by(|a, b| {
        let sa = scoped_hit_score(q, &terms, a);
        let sb = scoped_hit_score(q, &terms, b);
        sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
    });
    unique.truncate(limit);
    unique
}

fn scoped_hit_score(q: &str, terms: &[String], hit: &Hit) -> f64 {
    let q_lower = q.to_ascii_lowercase();
    let blob = format!(
        "{} {} {}",
        hit.title.as_deref().unwrap_or(""),
        hit.uri.as_deref().unwrap_or(""),
        hit.text
    )
    .to_ascii_lowercase();
    let exact = if blob.contains(&q_lower) { 1000.0 } else { 0.0 };
    let overlap = terms
        .iter()
        .filter(|term| blob.contains(term.as_str()))
        .count() as f64;
    exact + overlap * 10.0 + hit.score
}

fn query_terms(q: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut token = String::new();
    for c in q.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            token.push(c.to_ascii_lowercase());
        } else if token.len() > 1 {
            if seen.insert(token.clone()) {
                terms.push(std::mem::take(&mut token));
            } else {
                token.clear();
            }
        } else {
            token.clear();
        }
        if terms.len() >= 16 {
            break;
        }
    }
    if token.len() > 1 && terms.len() < 16 && seen.insert(token.clone()) {
        terms.push(token);
    }
    terms
}

fn get_cached_hits(
    state: &State,
    mode: SearchMode,
    q: &str,
    limit: usize,
    embed_query: bool,
) -> Option<Vec<Hit>> {
    let key = search_cache_key(mode, q, limit, embed_query);
    state.query_cache.lock().get(&key).cloned()
}

fn put_cached_hits(
    state: &State,
    mode: SearchMode,
    q: &str,
    limit: usize,
    embed_query: bool,
    hits: &[Hit],
) {
    let key = search_cache_key(mode, q, limit, embed_query);
    let mut cache = state.query_cache.lock();
    if cache.len() >= QUERY_CACHE_MAX {
        cache.clear();
    }
    cache.insert(key, hits.to_vec());
}

fn get_cached_embedding(state: &State, q: &str) -> Option<Vec<f32>> {
    state.embed_cache.lock().get(q).cloned()
}

fn put_cached_embedding(state: &State, q: &str, embedding: &[f32]) {
    let mut cache = state.embed_cache.lock();
    if cache.len() >= EMBED_CACHE_MAX {
        cache.clear();
    }
    cache.insert(q.to_string(), embedding.to_vec());
}

fn clear_query_cache(state: &State) {
    state.query_cache.lock().clear();
}

fn adjust_stats_cache_after_put(state: &State, id: i64, has_vec: bool) {
    let mut cache = state.stats_cache.lock();
    if let Some((at, docs, vecs, max_id)) = cache.as_mut() {
        *at = Instant::now();
        if id > *max_id {
            *docs += 1;
            *vecs += i64::from(has_vec);
            *max_id = id;
        }
    }
}

fn adjust_stats_cache_after_batch(state: &State, ids: &[i64], reqs: &[PutRequest]) {
    let mut cache = state.stats_cache.lock();
    if let Some((at, docs, vecs, max_id)) = cache.as_mut() {
        *at = Instant::now();
        for (id, req) in ids.iter().zip(reqs.iter()) {
            if *id > *max_id {
                *docs += 1;
                *vecs += i64::from(req.embedding.is_some());
                *max_id = *id;
            }
        }
    }
}

fn current_max_doc_id(state: &State) -> Result<i64> {
    let guard = state.sql_conn.lock();
    let conn = guard
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("sql_conn unavailable"))?;
    Ok(conn.query_row("SELECT COALESCE(MAX(id),0) FROM docs", [], |r| r.get(0))?)
}

fn fast_search_from_state(
    state: &State,
    mode: SearchMode,
    q: &str,
    emb: Option<&[f32]>,
    limit: usize,
) -> Result<Option<Vec<Hit>>> {
    if std::env::var("SYNAPSE_DISABLE_FAST_SEARCH").ok().as_deref() == Some("1") {
        return Ok(None);
    }
    let limit = limit.max(1);
    let k = limit.saturating_mul(3).max(limit);
    match mode {
        SearchMode::Lex => Ok(fast_lex_from_state(state, q, limit)?
            .map(Some)
            .unwrap_or(None)),
        SearchMode::Vec => {
            let Some(emb) = emb else {
                return Ok(None);
            };
            fast_vec_from_state(state, emb, limit)
        }
        SearchMode::Hybrid => {
            let Some(emb) = emb else {
                return Ok(None);
            };
            let lex = fast_lex_from_state(state, q, k)?.unwrap_or_default();
            let vec = fast_vec_from_state(state, emb, k)?.unwrap_or_default();
            if lex.is_empty() && vec.is_empty() {
                Ok(None)
            } else {
                Ok(Some(rrf_merge_neon(lex, vec, limit)))
            }
        }
    }
}

fn fast_lex_from_state(state: &State, q: &str, limit: usize) -> Result<Option<Vec<Hit>>> {
    let Some(fts_query) = fts5_match_query(q) else {
        return Ok(Some(Vec::new()));
    };
    let guard = state.sql_conn.lock();
    let Some(conn) = guard.as_ref() else {
        return Ok(None);
    };
    let use_bm25 = should_rank_lex_with_bm25(q);
    let sql = if use_bm25 {
        format!(
            "SELECT d.id,d.uri,d.title,substr(d.text,1,{SEARCH_SNIPPET_CHARS}),bm25(docs_fts) as score
             FROM docs_fts JOIN docs d ON d.id = docs_fts.rowid
             WHERE docs_fts MATCH ?1
             ORDER BY score LIMIT ?2"
        )
    } else {
        format!(
            "SELECT d.id,d.uri,d.title,substr(d.text,1,{SEARCH_SNIPPET_CHARS}),0.0 as score
             FROM docs_fts JOIN docs d ON d.id = docs_fts.rowid
             WHERE docs_fts MATCH ?1
             LIMIT ?2"
        )
    };
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt.query_map(params![fts_query, limit as i64], |r| {
        let raw_score = r.get::<_, f64>(4)?;
        Ok(Hit {
            id: r.get(0)?,
            uri: r.get(1)?,
            title: r.get(2)?,
            text: r.get(3)?,
            score: if use_bm25 { -raw_score } else { raw_score },
            meta: None,
            ts: None,
        })
    })?;
    Ok(Some(rows.collect::<rusqlite::Result<_>>()?))
}

fn fts5_match_query(q: &str) -> Option<String> {
    let trimmed = q.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c.is_ascii_whitespace())
    {
        return Some(trimmed.to_string());
    }

    let mut tokens = Vec::new();
    let mut token = String::new();
    for c in trimmed.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            token.push(c);
        } else if token.len() > 1 {
            tokens.push(std::mem::take(&mut token));
        } else {
            token.clear();
        }
        if tokens.len() >= 16 {
            break;
        }
    }
    if token.len() > 1 && tokens.len() < 16 {
        tokens.push(token);
    }
    if tokens.is_empty() {
        None
    } else {
        Some(
            tokens
                .into_iter()
                .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(" OR "),
        )
    }
}

fn fts5_loose_match_query(q: &str) -> Option<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    for c in q.trim().chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            token.push(c);
        } else if token.len() > 1 {
            tokens.push(std::mem::take(&mut token));
        } else {
            token.clear();
        }
        if tokens.len() >= 16 {
            break;
        }
    }
    if token.len() > 1 && tokens.len() < 16 {
        tokens.push(token);
    }
    if tokens.is_empty() {
        None
    } else {
        Some(
            tokens
                .into_iter()
                .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(" OR "),
        )
    }
}

fn should_rank_lex_with_bm25(q: &str) -> bool {
    let mut token_count = 0usize;
    for token in q.split_whitespace() {
        if token.chars().any(|c| c.is_ascii_alphanumeric()) {
            token_count += 1;
            if token_count > 1 {
                return true;
            }
        }
    }
    false
}

fn fast_vec_from_state(state: &State, emb: &[f32], limit: usize) -> Result<Option<Vec<Hit>>> {
    let pairs = {
        let guard = state.ndarray_idx.read();
        let Some(ref idx) = *guard else {
            return Ok(None);
        };
        if idx.is_empty() {
            return Ok(None);
        }
        let binary_k = 4096usize.max(limit * 64).min(idx.len());
        if binary_k < idx.len() {
            idx.search_cascade(emb, limit, binary_k)
        } else {
            idx.search(emb, limit)
        }
    };
    if pairs.is_empty() {
        return Ok(None);
    }
    hydrate_pairs_from_state(state, &pairs).map(Some)
}

fn hydrate_pairs_from_state(state: &State, pairs: &[(i64, f32)]) -> Result<Vec<Hit>> {
    if pairs.is_empty() {
        return Ok(Vec::new());
    }
    let guard = state.sql_conn.lock();
    let conn = guard
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("sql_conn unavailable"))?;
    let placeholders = (0..pairs.len())
        .map(|i| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT id,uri,title,substr(text,1,{SEARCH_SNIPPET_CHARS}) FROM docs WHERE id IN ({placeholders})"
    );
    let ids: Vec<i64> = pairs.iter().map(|(id, _)| *id).collect();
    let params_iter: Vec<&dyn rusqlite::ToSql> =
        ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut by_id: DocProjectionMap = Default::default();
    let rows = stmt.query_map(params_iter.as_slice(), |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    for row in rows {
        let (id, uri, title, text) = row?;
        by_id.insert(id, (uri, title, text));
    }
    let mut out = Vec::with_capacity(pairs.len());
    for (id, dist) in pairs {
        if let Some((uri, title, text)) = by_id.remove(id) {
            out.push(Hit {
                id: *id,
                uri,
                title,
                text,
                score: 1.0_f64 / (1.0_f64 + *dist as f64),
                meta: None,
                ts: None,
            });
        }
    }
    Ok(out)
}
