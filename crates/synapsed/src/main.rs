//! synapsed — persistent daemon. Unix socket + length-prefixed msgpack.

// mimalloc — 5-15% faster than system malloc for many small allocs (msgpack, vec ops).
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod metrics;
mod proto;
mod livequery;

use anyhow::{Context, Result};
use clap::Parser;
use proto::{PutReq, Request, Response};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use synapse_core::{embed::Embedder, embedder_trait::TextEmbedder, snap, PutRequest, SearchMode, Store};
use synapse_core::turbo::ndarray_search::NdArraySearch;
use synapse_rerank::{build_reranker_from_env, IdentityReranker, Reranker};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;
use tokio::time::timeout;
use parking_lot::Mutex as PlMutex;
use tracing::{error, info, warn};

/// Idle timeout between requests on a kept-alive socket. Prevents fd leaks
/// from clients that connect but never send (or never disconnect cleanly).
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

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
    #[arg(long, default_value = "0000000000000000000000000000000000000000000000000000000000000000")]
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
    embedder: Mutex<Option<Box<dyn TextEmbedder>>>,
    reranker: Box<dyn Reranker>,
    db_path: PathBuf,
    cache_path: PathBuf,
    snap_dir: PathBuf,
    max_put_bytes: usize,
    /// Read-only PRAGMA-tuned connection for Sql op. Avoid per-call open/PRAGMA overhead.
    sql_conn: PlMutex<Option<rusqlite::Connection>>,
    /// LiveQuery broadcast broker (P2.2). Emits on Put/PutBatch/Merge.
    live_broker: livequery::LiveBroker,
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
            *g = Some(synapse_core::embed::pick_embedder_with_cache(Some(&self.cache_path)));
            info!("embedder ready in {:?}", t0.elapsed());
        }
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
                    warn!("failed to read license file {}: {e} — falling back to free tier", jwt_path.display());
                    return Store::open(file).context("open store (free tier)");
                }
            };
            let pubkey_bytes = match hex::decode(pubkey_hex) {
                Ok(b) if b.len() == 32 => b,
                _ => {
                    warn!("invalid --license-pubkey (must be 32-byte hex) — falling back to free tier");
                    return Store::open(file).context("open store (free tier)");
                }
            };
            match synapse_license::verify_license(&jwt, &pubkey_bytes) {
                Ok(license) => {
                    info!("license verified: customer={} tier={}", license.customer_id, license.tier);
                    let hw_fp = synapse_license::current_hw_fingerprint();
                    // Use the raw JWT bytes as the "signature" material for brain_key derivation.
                    let brain_key = synapse_core::db::derive_brain_key(jwt.as_bytes(), &hw_fp);
                    info!("brain_key derived — opening encrypted store");
                    return Store::open_with_brain_key(file, &brain_key).context("open encrypted store");
                }
                Err(e) => {
                    warn!("license invalid ({e}) — falling back to free tier (unencrypted)");
                    return Store::open(file).context("open store (free tier)");
                }
            }
        } else {
            info!("no license file found at {} — running free tier", jwt_path.display());
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

    let mut store = open_store(&cli.file, &cli.license_jwt, &cli.license_pubkey)?;
    // Pre-warm the turbo ndarray search engine (loads 164K vectors, ~1.7s).
    // Must happen BEFORE the server starts accepting requests, otherwise the
    // first search blocks the tokio async runtime for ~2 seconds.
    store.warm_turbo();
    // Lift the warmed NdArraySearch out of the Store mutex into a top-level
    // Arc<RwLock<>> so concurrent vec searches bypass the Store mutex entirely.
    // Reads acquire a shared read-lock (non-exclusive); writes still go through
    // the store mutex (rare) and also update this index under write-lock.
    let ndarray_idx = Arc::new(parking_lot::RwLock::new(store.take_ndarray_search()));
    let cache_path = cli.emb_cache.clone().unwrap_or_else(|| {
        let mut p = cli.file.clone();
        let name = p
            .file_name()
            .map(|n| format!(".{}.emb-cache", n.to_string_lossy()))
            .unwrap_or_else(|| ".emb-cache".into());
        p.set_file_name(name);
        p
    });
    let embedder: Option<Box<dyn TextEmbedder>> = if cli.lazy_embed {
        None
    } else {
        info!("warming embedder…");
        Some(synapse_core::embed::pick_embedder_with_cache(Some(&cache_path)))
    };
    let snap_dir = cli.snap_dir.clone().unwrap_or_else(|| {
        cli.file
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."))
    });
    std::fs::create_dir_all(&snap_dir).ok();
    let reranker: Box<dyn Reranker> = build_reranker(&cli.rerank_model);
    // Pre-open PRAGMA-tuned read-only Sql conn for analytics ops.
    let sql_conn = {
        use rusqlite::{Connection, OpenFlags};
        match Connection::open_with_flags(&cli.file, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI) {
            Ok(c) => {
                let _ = c.pragma_update(None, "mmap_size", 1_073_741_824_i64);
                let _ = c.pragma_update(None, "cache_size", -262_144_i64);
                let _ = c.pragma_update(None, "temp_store", 2_i64);
                Some(c)
            }
            Err(e) => { tracing::warn!("sql_conn pre-open failed: {e}"); None }
        }
    };
    let state = Arc::new(State {
        store: PlMutex::new(store),
        ndarray_idx,
        embedder: Mutex::new(embedder),
        reranker,
        db_path: cli.file.clone(),
        cache_path,
        snap_dir,
        max_put_bytes: cli.max_put_bytes,
        sql_conn: PlMutex::new(sql_conn),
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

    // SIGTERM/SIGINT handler — persist ANN sidecar before exit.
    // Saves 5min HNSW rebuild on next start.
    let sig_state = state.clone();
    tokio::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};
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

async fn handle_conn(mut stream: UnixStream, state: Arc<State>) -> Result<()> {
    let api_key = std::env::var("SYNAPSE_API_KEY").ok();
    let auth_required = api_key.is_some();
    let mut authed = !auth_required;  // session-scoped auth state
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
                    stream.write_all(&(encoded.len() as u32).to_le_bytes()).await?;
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
        stream.write_all(&(encoded.len() as u32).to_le_bytes()).await?;
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
                        op: "Put".into(), id, title, uri,
                        ts: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0),
                    });
                    Response::Id(id)
                }
                Err(e) => Response::Err(e.to_string()),
            }
        }
        Request::PutBatch(batch) => {
            let t0 = Instant::now();
            let titles: Vec<_> = batch.iter().map(|p| (p.title.clone(), p.uri.clone())).collect();
            let result = put_batch(state, batch).await;
            metrics::record_put(t0.elapsed());
            match result {
                Ok(ids) => {
                    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
                    for (id, (title, uri)) in ids.iter().zip(titles.iter()) {
                        state.live_broker.emit(livequery::LiveEvent {
                            op: "PutBatch".into(), id: *id, title: title.clone(), uri: uri.clone(), ts: now,
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
        Request::Stats => match { state.store.lock().stats() } {
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
        } => match { state.store.lock().merge_crdt(id, &crdt_state) } {
            Ok(()) => Response::Ok,
            Err(e) => Response::Err(e.to_string()),
        },
        Request::Timeline { limit, offset } => {
            match { state.store.lock().timeline(limit, offset) } {
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
                        match { state.store.lock().verify(id, &verifying_key) } {
                            Ok(()) => Response::Ok,
                            Err(e) => Response::Err(e.to_string()),
                        }
                    }
                },
            }
        }
        Request::Embed { text, dim } => {
            match embed_one(state, &text).await {
                Ok(mut vec) => {
                    // Matryoshka truncation: if dim provided and < native, truncate + L2-renorm.
                    if let Some(d) = dim {
                        if d < vec.len() && d > 0 {
                            vec.truncate(d);
                            let n: f32 = vec.iter().map(|x| x*x).sum::<f32>().sqrt();
                            if n > 1e-10 { for x in vec.iter_mut() { *x /= n; } }
                        }
                    }
                    Response::Embed { vec }
                }
                Err(e) => Response::Err(e.to_string()),
            }
        }
        Request::SnapMerge { snapshot_path, out_path, level } => {
            let db_path = state.db_path.clone();
            let tmp = std::env::temp_dir().join(format!("synapse-snap-{}.brainpack", std::process::id()));
            match synapse_core::snap::export(&db_path, &tmp, level)
                .and_then(|_| synapse_core::snap::merge_packs(&tmp, std::path::Path::new(&snapshot_path), std::path::Path::new(&out_path), level))
            {
                Ok(_) => { let _ = std::fs::remove_file(&tmp); Response::Ok }
                Err(e) => { let _ = std::fs::remove_file(&tmp); Response::Err(e.to_string()) }
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
                // Hydrate id→text under Store lock (brief SQL round-trip).
                let result = tokio::task::block_in_place(|| {
                    let store = state.store.lock();
                    store.hydrate_hits_by_id_dist(&pairs)
                });
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
        Request::Rerank { query, candidates, top_k } => {
            match state.reranker.rerank(&query, candidates, top_k) {
                Ok(hits) => Response::Hits(hits),
                Err(e) => Response::Err(e.to_string()),
            }
        }
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
            if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
                return Response::Err("invalid tenant name (alnum/_/- only)".into());
            }
            let path = format!("{}/.synapse/tenants/{}.db", std::env::var("HOME").unwrap_or_default(), name);
            if !std::path::Path::new(&path).exists() {
                return Response::Err(format!("tenant db not found: {path}"));
            }
            let result: std::result::Result<(), String> = tokio::task::block_in_place(|| {
                let mut g = state.sql_conn.lock();
                if let Some(conn) = g.as_mut() {
                    conn.execute(&format!("DETACH DATABASE tenant"), []).ok();
                    conn.execute(&format!("ATTACH DATABASE 'file:{}?mode=ro' AS tenant", path), [])
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
                    if subtle_eq(&token, &k) { Response::Ok } else { Response::Err("invalid token".into()) }
                }
                _ => Response::Ok,  // no key configured — auth always passes
            }
        }
        Request::Transaction { ops } => {
            // Atomic batch via put_batch (single SQL transaction in store).
            let put_reqs: Vec<PutRequest> = ops.into_iter().map(|p| p.into()).collect();
            let result = tokio::task::block_in_place(|| {
                let mut store = state.store.lock();
                store.put_batch(&put_reqs)
            });
            match result {
                Ok(ids) => Response::Ids(ids),
                Err(e) => Response::Err(e.to_string()),
            }
        }
        Request::Sql { query, params } => {
            // Read-only raw SQL via pooled PRAGMA-tuned conn (avoid per-call open + PRAGMA cost).
            let result: Result<(Vec<String>, Vec<Vec<serde_json::Value>>), String> = tokio::task::block_in_place(|| {
                use rusqlite::types::ValueRef;
                let guard = state.sql_conn.lock();
                let conn = guard.as_ref().ok_or_else(|| "sql_conn unavailable".to_string())?;
                let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;
                let cols: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
                let n_cols = cols.len();
                let rusq_params: Vec<rusqlite::types::Value> = params.iter().map(|v| {
                    match v {
                        serde_json::Value::Null => rusqlite::types::Value::Null,
                        serde_json::Value::Bool(b) => rusqlite::types::Value::Integer(*b as i64),
                        serde_json::Value::Number(n) => {
                            if let Some(i) = n.as_i64() { rusqlite::types::Value::Integer(i) }
                            else if let Some(f) = n.as_f64() { rusqlite::types::Value::Real(f) }
                            else { rusqlite::types::Value::Null }
                        }
                        serde_json::Value::String(s) => rusqlite::types::Value::Text(s.clone()),
                        _ => rusqlite::types::Value::Text(v.to_string()),
                    }
                }).collect();
                let param_refs: Vec<&dyn rusqlite::ToSql> = rusq_params.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
                let mut rows_out = Vec::new();
                let mut rows_iter = stmt.query(rusqlite::params_from_iter(&param_refs)).map_err(|e| e.to_string())?;
                while let Some(row) = rows_iter.next().map_err(|e| e.to_string())? {
                    let mut row_vals = Vec::with_capacity(n_cols);
                    for i in 0..n_cols {
                        let v = row.get_ref(i).map_err(|e| e.to_string())?;
                        row_vals.push(match v {
                            ValueRef::Null => serde_json::Value::Null,
                            ValueRef::Integer(i) => serde_json::Value::from(i),
                            ValueRef::Real(f) => serde_json::Value::from(f),
                            ValueRef::Text(t) => serde_json::Value::String(String::from_utf8_lossy(t).into_owned()),
                            ValueRef::Blob(b) => serde_json::Value::String(format!("<blob:{}b>", b.len())),
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
    if a.len() != b.len() { return false; }
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
    tokio::task::block_in_place(|| {
        let mut store = state.store.lock();
        Ok(store.put(&req)?)
    })
}

async fn put_batch(state: &State, batch: Vec<PutReq>) -> Result<Vec<i64>> {
    for r in &batch {
        if r.text.len() > state.max_put_bytes {
            anyhow::bail!("text too large: {} > {}", r.text.len(), state.max_put_bytes);
        }
    }
    let need_server_embed: Vec<bool> = batch.iter().map(|r| r.embed && r.embedding.is_none()).collect();
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
    tokio::task::block_in_place(|| {
        let mut store = state.store.lock();
        Ok(store.put_batch(&reqs)?)
    })
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
    let q_owned = q.to_owned();
    let emb_owned = emb;
    let (hits, latency_us, hit_count, top_score, log_err) =
        tokio::task::block_in_place(|| {
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
    Ok(hits)
}
