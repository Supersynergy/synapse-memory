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

/// Returns number of ONNX sessions: half of logical cores, min 2.
/// On M4 Max (12 cores) → 6 sessions. Replaces old `POOL_SIZE = 2`.
fn get_pool_size() -> usize {
    std::thread::available_parallelism()
        .map(|n| (n.get() / 2).max(2))
        .unwrap_or(4)
}

/// Global pool of pre-warmed TextEmbedding sessions.
static SESSION_POOL: OnceCell<Mutex<Vec<TextEmbedding>>> = OnceCell::new();

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

/// Absolute ONNX-model cache dir, resolved independently of the current working
/// directory. fastembed's default is `.fastembed_cache` relative to the cwd, which
/// fails ("Failed to retrieve onnx/model.onnx") when a CLI/agent runs from any other
/// dir. Honor the daemon's env first, else a stable abs path under $HOME.
fn embed_cache_dir() -> std::path::PathBuf {
    for key in ["FASTEMBED_CACHE_PATH", "HF_HOME"] {
        if let Some(v) = std::env::var_os(key)
            && !v.is_empty()
        {
            return std::path::PathBuf::from(v);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        return std::path::PathBuf::from(home)
            .join(".synapse")
            .join(".fastembed_cache");
    }
    std::path::PathBuf::from(".fastembed_cache")
}

fn get_or_init_pool() -> Result<&'static Mutex<Vec<TextEmbedding>>> {
    SESSION_POOL.get_or_try_init(|| {
        let pool_size = get_pool_size();
        let model = select_model();
        let cache_dir = embed_cache_dir();
        std::fs::create_dir_all(&cache_dir).ok();
        let mut sessions = Vec::with_capacity(pool_size);
        for _ in 0..pool_size {
            let m = TextEmbedding::try_new(
                InitOptions::new(model.clone())
                    .with_show_download_progress(false)
                    .with_cache_dir(cache_dir.clone()),
            )
            .map_err(|e| Error::Other(format!("fastembed init: {e}")))?;
            sessions.push(m);
        }
        Ok(Mutex::new(sessions))
    })
}

/// Warm the global ONNX session pool eagerly (call once at daemon start).
pub fn warm_pool() -> Result<()> {
    get_or_init_pool()?;
    Ok(())
}

pub struct Embedder {
    cache: Option<Arc<Database>>,
}

impl Embedder {
    pub fn new() -> Result<Self> {
        get_or_init_pool()?;
        Ok(Self { cache: None })
    }

    pub fn new_with_cache<P: AsRef<Path>>(cache_path: Option<P>) -> Result<Self> {
        get_or_init_pool()?;
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
        let pool = get_or_init_pool()?;
        // Acquire lock only to pop — drop guard before ONNX inference (~5-15ms).
        let mut session = {
            let mut guard = pool.lock();
            guard
                .pop()
                .ok_or_else(|| Error::Other("pool empty".into()))?
        };
        let result = session
            .embed(texts, None)
            .map_err(|e| Error::Other(format!("embed: {e}")));
        // Re-acquire to push back.
        pool.lock().push(session);
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
    // SYNAPSE_DISABLE_MLX=1 forces the CPU fastembed path (skip the GPU/Metal sidecar).
    // Benchmark 2026-06-16: MLX gives ~0% recall gain (search-dominated) and only ~17%
    // ingest gain; CPU starts faster (1.3s vs 3.1s) with no 5s sidecar-timeout flake.
    // Set this for low-GPU operation; unset to re-enable Metal for bulk re-index.
    let _mlx_disabled = std::env::var("SYNAPSE_DISABLE_MLX").as_deref() == Ok("1");
    #[cfg(all(target_os = "macos", target_arch = "aarch64", feature = "embed-mlx"))]
    if !_mlx_disabled {
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
