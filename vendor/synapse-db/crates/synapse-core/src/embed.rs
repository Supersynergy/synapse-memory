//! Embedding pipeline: fastembed-rs (BGE-small-en-v1.5 ONNX, 384-dim) + redb BLAKE3 cache.
//!
//! Cold-start fix: global ONNX session pool (default 2 sessions) initialized once,
//! reused across all Embedder instances — eliminates per-request model reload overhead.

use crate::error::{Error, Result};
#[cfg(feature = "embed")]
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
#[cfg(all(feature = "embed-dynamic", not(feature = "embed")))]
use fastembed_dynamic::{EmbeddingModel, InitOptions, TextEmbedding};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use rayon::prelude::*;
use redb::{Database, ReadableTableMetadata, TableDefinition};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

const EMB_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("emb_cache_v1");

/// PR-D1 scale-100M: process-wide observability counters for the embed cache.
static CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static CACHE_MISSES: AtomicU64 = AtomicU64::new(0);

/// Returns (hits, misses) since process start.
pub fn cache_counters() -> (u64, u64) {
    (
        CACHE_HITS.load(Ordering::Relaxed),
        CACHE_MISSES.load(Ordering::Relaxed),
    )
}

/// Max number of ONNX sessions: half of logical cores, min 2.
/// Overridable via `SYNAPSE_EMBED_POOL`. Sessions are created lazily —
/// the cap is a ceiling, not eager cost (bd -frt): a one-shot `synx put`
/// pays a single model load instead of `cores/2`.
fn get_pool_size() -> usize {
    std::env::var("SYNAPSE_EMBED_POOL")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| (n.get() / 2).max(2))
                .unwrap_or(4)
        })
}

/// Lazily-grown session pool: `idle` holds reusable sessions, `total`
/// counts created+in-flight ones against `get_pool_size()`.
struct SessionPool {
    idle: Vec<TextEmbedding>,
    total: usize,
}

static SESSION_POOL: OnceCell<(Mutex<SessionPool>, parking_lot::Condvar)> = OnceCell::new();

fn session_pool() -> &'static (Mutex<SessionPool>, parking_lot::Condvar) {
    SESSION_POOL.get_or_init(|| {
        (
            Mutex::new(SessionPool {
                idle: Vec::new(),
                total: 0,
            }),
            parking_lot::Condvar::new(),
        )
    })
}

fn new_session() -> Result<TextEmbedding> {
    TextEmbedding::try_new(InitOptions::new(select_model()).with_show_download_progress(false))
        .map_err(|e| Error::Other(format!("fastembed init: {e}")))
}

/// Take a session: reuse an idle one, or create one under the cap
/// (creation happens outside the lock), or wait for a release.
fn acquire_session() -> Result<TextEmbedding> {
    let (m, cv) = session_pool();
    let mut g = m.lock();
    loop {
        if let Some(s) = g.idle.pop() {
            return Ok(s);
        }
        if g.total < get_pool_size() {
            g.total += 1; // reserve a slot before the slow init
            break;
        }
        cv.wait(&mut g);
    }
    drop(g);
    match new_session() {
        Ok(s) => Ok(s),
        Err(e) => {
            session_pool().0.lock().total -= 1;
            Err(e)
        }
    }
}

fn release_session(s: TextEmbedding) {
    let (m, cv) = session_pool();
    m.lock().idle.push(s);
    cv.notify_one();
}

/// Select embedding model from `SYNAPSE_EMBED_MODEL` env-var.
/// Default `bge-small` (384-dim, MTEB 53.0) for backward compatibility.
///
/// IMPORTANT: switching models invalidates existing vector corpora (different dim).
/// Use a fresh `.synapse/` directory after switching.
///
/// Accepted values:
///   `bge-small`   → BGESmallENV15            (384-dim, MTEB 53.0, default)
///   `bge-small-q` → BGESmallENV15Q           (384-dim, int8 quantized, smaller)
///   `arctic-xs`   → SnowflakeArcticEmbedXS   (384-dim, MTEB 56.6)
///   `arctic-s`    → SnowflakeArcticEmbedS    (384-dim, MTEB 60.0)
///   `arctic-m`    → SnowflakeArcticEmbedM    (768-dim, MTEB 62.5) ← upgrade target
///   `arctic-l`    → SnowflakeArcticEmbedL    (1024-dim, MTEB 63.0)
///   `mxbai-large` → MxbaiEmbedLargeV1        (1024-dim, MTEB 64.7)
///   `nomic-1.5`   → NomicEmbedTextV15        (768-dim, MTEB 62.4)
fn select_model() -> EmbeddingModel {
    match std::env::var("SYNAPSE_EMBED_MODEL")
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "bge-small-q" => EmbeddingModel::BGESmallENV15Q,
        "arctic-xs" => EmbeddingModel::SnowflakeArcticEmbedXS,
        "arctic-s" => EmbeddingModel::SnowflakeArcticEmbedS,
        "arctic-m" => EmbeddingModel::SnowflakeArcticEmbedM,
        "arctic-l" => EmbeddingModel::SnowflakeArcticEmbedL,
        "mxbai-large" => EmbeddingModel::MxbaiEmbedLargeV1,
        "nomic-1.5" => EmbeddingModel::NomicEmbedTextV15,
        _ => EmbeddingModel::BGESmallENV15,
    }
}

/// Warm the session pool eagerly up to the cap — call once at daemon
/// start so request latency doesn't pay model-init cost. One-shot CLI
/// paths skip this and grow lazily instead (bd -frt).
pub fn warm_pool() -> Result<()> {
    let target = get_pool_size();
    let needed = target.saturating_sub(session_pool().0.lock().total);
    let mut made = Vec::with_capacity(needed);
    for _ in 0..needed {
        made.push(new_session()?);
    }
    let mut g = session_pool().0.lock();
    g.total += made.len();
    g.idle.extend(made);
    Ok(())
}

pub struct Embedder {
    cache: Option<Arc<Database>>,
}

impl Embedder {
    pub fn new() -> Result<Self> {
        // No eager model load — sessions materialize on first embed (bd -frt).
        Ok(Self { cache: None })
    }

    pub fn new_with_cache<P: AsRef<Path>>(cache_path: Option<P>) -> Result<Self> {
        let cache = match cache_path {
            Some(p) => {
                if let Some(parent) = p.as_ref().parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                let db = Database::create(p.as_ref())
                    .map_err(|e| Error::Other(format!("redb create: {e}")))?;
                let wtx = db
                    .begin_write()
                    .map_err(|e| Error::Other(format!("redb wtx: {e}")))?;
                {
                    let _ = wtx
                        .open_table(EMB_TABLE)
                        .map_err(|e| Error::Other(format!("redb open: {e}")))?;
                }
                wtx.commit()
                    .map_err(|e| Error::Other(format!("redb commit: {e}")))?;
                Some(Arc::new(db))
            }
            None => None,
        };
        Ok(Self { cache })
    }

    fn embed_raw(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        // Lazy acquire: reuse idle session, create under cap, or wait (bd -frt).
        let mut session = acquire_session()?;
        let result = session
            .embed(texts, None)
            .map_err(|e| Error::Other(format!("embed: {e}")));
        release_session(session);
        result
    }

    pub fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if let Some(ref cache) = self.cache {
            return self.embed_batch_cached(cache, texts);
        }
        self.embed_raw(texts.to_vec())
    }

    fn embed_batch_cached(&self, cache: &Database, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        // PR-D1 scale-100M: parallelize BLAKE3 hashing via rayon — but ONLY when
        // the batch is large enough for parallelism to pay for itself.
        // Measured on M4 Max: N=1000 rayon 1.9× SLOWER, N=10000 rayon 1.94× FASTER.
        // Empirical break-even ≈ 5000. Stay serial below that.
        const RAYON_HASH_THRESHOLD: usize = 5_000;
        let hashes: Vec<[u8; 32]> = if texts.len() >= RAYON_HASH_THRESHOLD {
            texts
                .par_iter()
                .map(|t| *blake3::hash(t.as_bytes()).as_bytes())
                .collect()
        } else {
            texts
                .iter()
                .map(|t| *blake3::hash(t.as_bytes()).as_bytes())
                .collect()
        };
        let mut out: Vec<Option<Vec<f32>>> = vec![None; texts.len()];
        let mut miss_idx: Vec<usize> = Vec::new();
        {
            let rtx = cache
                .begin_read()
                .map_err(|e| Error::Other(format!("redb rtx: {e}")))?;
            let t = rtx
                .open_table(EMB_TABLE)
                .map_err(|e| Error::Other(format!("redb tbl: {e}")))?;
            for (i, h) in hashes.iter().enumerate() {
                if let Some(v) = t
                    .get(h.as_slice())
                    .map_err(|e| Error::Other(format!("redb get: {e}")))?
                {
                    let bytes = v.value();
                    let mut v = Vec::with_capacity(bytes.len() / 4);
                    for chunk in bytes.chunks_exact(4) {
                        v.push(f32::from_le_bytes(chunk.try_into().unwrap()));
                    }
                    out[i] = Some(v);
                } else {
                    miss_idx.push(i);
                }
            }
        }
        CACHE_HITS.fetch_add((texts.len() - miss_idx.len()) as u64, Ordering::Relaxed);
        CACHE_MISSES.fetch_add(miss_idx.len() as u64, Ordering::Relaxed);
        if !miss_idx.is_empty() {
            let miss_texts: Vec<String> = miss_idx.iter().map(|&i| texts[i].clone()).collect();
            let new_embs = self.embed_raw(miss_texts)?;
            // PR-D1: f32→LE-bytes packing measured SLOWER with rayon at all sizes.
            // Allocation dominates per-row. Keep serial.
            let byte_rows: Vec<Vec<u8>> = new_embs
                .iter()
                .map(|emb| emb.iter().flat_map(|f| f.to_le_bytes()).collect())
                .collect();
            let wtx = cache
                .begin_write()
                .map_err(|e| Error::Other(format!("redb wtx: {e}")))?;
            {
                let mut t = wtx
                    .open_table(EMB_TABLE)
                    .map_err(|e| Error::Other(format!("redb tbl: {e}")))?;
                for ((emb, bytes), &i) in new_embs.iter().zip(byte_rows.iter()).zip(miss_idx.iter())
                {
                    t.insert(hashes[i].as_slice(), bytes.as_slice())
                        .map_err(|e| Error::Other(format!("redb ins: {e}")))?;
                    out[i] = Some(emb.clone());
                }
            }
            wtx.commit()
                .map_err(|e| Error::Other(format!("redb commit: {e}")))?;
        }
        Ok(out.into_iter().map(|o| o.unwrap()).collect())
    }

    pub fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        let mut out = self.embed_batch(&[text.to_string()])?;
        out.pop().ok_or_else(|| Error::Other("empty embed".into()))
    }

    pub fn cache_stats(&self) -> Result<Option<u64>> {
        let Some(ref c) = self.cache else {
            return Ok(None);
        };
        let rtx = c.begin_read().map_err(|e| Error::Other(format!("{e}")))?;
        let t = rtx
            .open_table(EMB_TABLE)
            .map_err(|e| Error::Other(format!("{e}")))?;
        Ok(Some(t.len().map_err(|e| Error::Other(format!("{e}")))?))
    }
}

/// Pick the best available embedder backend at runtime.
///
/// On Apple Silicon with `embed-mlx` + `turbo` features: attempts MLX Metal,
/// falls back to fastembed on error. Everywhere else: returns fastembed ONNX CPU.
#[cfg(feature = "turbo")]
pub fn pick_embedder() -> Box<dyn crate::embedder_trait::TextEmbedder> {
    pick_embedder_with_cache::<&std::path::Path>(None)
}

/// Variant that lets the caller supply a fastembed cache path. The MLX path
/// has no equivalent cache concept (sidecar handles model load itself), so
/// the cache argument is only consumed by the fastembed fallback.
#[cfg(feature = "turbo")]
pub fn pick_embedder_with_cache<P: AsRef<std::path::Path>>(
    cache_path: Option<P>,
) -> Box<dyn crate::embedder_trait::TextEmbedder> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64", feature = "embed-mlx"))]
    {
        use crate::embed_mlx::MlxMetalEmbedder;
        match MlxMetalEmbedder::new() {
            Ok(mlx) => {
                tracing::info!(
                    backend = "mlx-metal",
                    model = "bge-small-en-v1.5-bf16",
                    "pick_embedder: MLX Metal selected (Apple Silicon)"
                );
                return Box::new(mlx);
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "pick_embedder: MLX sidecar init failed, falling back to fastembed CPU"
                );
            }
        }
    }
    tracing::info!(
        backend = "fastembed-onnx-cpu",
        model = "bge-small-en-v1.5",
        "pick_embedder: fastembed ONNX CPU selected"
    );
    Box::new(Embedder::new_with_cache(cache_path).expect("fastembed pool init failed"))
}
