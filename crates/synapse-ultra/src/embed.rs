use std::path::Path;
use std::sync::Mutex;

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use lru::LruCache;
use rusqlite::Connection;

use crate::embed_mlx::MlxEmbedder;
use crate::error::{Result, UltraError};
use crate::snapshot::{normalize_vec, EMBED_DIM};

const MEM_CACHE_CAP: usize = 4096;

/// Whether to try MLX as primary embedder.
/// Enabled when `ULTRA_EMBEDDER=mlx` OR on macOS aarch64 (try-first, fallback).
fn mlx_enabled() -> bool {
    if let Ok(v) = std::env::var("ULTRA_EMBEDDER") {
        return v.eq_ignore_ascii_case("mlx");
    }
    // Auto-enable on Apple Silicon; disable with ULTRA_EMBEDDER=fastembed
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

pub struct Embedder {
    model: Mutex<Option<TextEmbedding>>,
    mem_cache: Mutex<LruCache<[u8; 32], Vec<f32>>>,
    db: Mutex<Option<Connection>>,
    mlx: Option<MlxEmbedder>,
}

impl Embedder {
    pub fn new(emb_cache_path: Option<&Path>) -> Result<Self> {
        let db = emb_cache_path
            .and_then(|p| Connection::open(p).ok());
        if let Some(ref db) = db {
            db.execute_batch(
                "CREATE TABLE IF NOT EXISTS emb_cache (
                    query_hash TEXT PRIMARY KEY,
                    query_text TEXT NOT NULL,
                    embedding BLOB NOT NULL,
                    created_at INTEGER NOT NULL DEFAULT (unixepoch())
                )",
            )
            .ok();
        }
        Ok(Embedder {
            model: Mutex::new(None),
            mem_cache: Mutex::new(LruCache::new(
                std::num::NonZeroUsize::new(MEM_CACHE_CAP).unwrap(),
            )),
            db: Mutex::new(db),
            mlx: MlxEmbedder::new().ok(),
        })
    }

    pub fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let key = Self::hash_key(text);

        // T0: in-mem LRU
        {
            let mut mc = self.mem_cache.lock().unwrap();
            if let Some(v) = mc.get(&key) {
                return Ok(v.clone());
            }
        }

        // T1: sqlite cache
        {
            let db_lock = self.db.lock().unwrap();
            if let Some(ref db) = *db_lock {
                let hex = hex_key(&key);
                let result: rusqlite::Result<Vec<u8>> = db.query_row(
                    "SELECT embedding FROM emb_cache WHERE query_hash = ?1",
                    rusqlite::params![hex],
                    |row| row.get(0),
                );
                if let Ok(blob) = result {
                    if blob.len() == EMBED_DIM * 4 {
                        let mut v: Vec<f32> = blob
                            .chunks_exact(4)
                            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                            .collect();
                        self.mem_cache.lock().unwrap().put(key, v.clone());
                        return Ok(v);
                    }
                }
            }
        }

        // T2: MLX Metal (Apple Silicon, fast path)
        let mut v = if mlx_enabled() {
            match self.mlx.as_ref().map(|m| m.embed_one(text)).unwrap_or_else(|| Err(UltraError::Embed("mlx unavailable".into()))) {
                Ok(vec) => vec,
                Err(e) => {
                    tracing::debug!("mlx embed failed, falling back to fastembed: {e}");
                    let mut fv = self.embed_raw(text)?;
                    normalize_vec(&mut fv);
                    fv
                }
            }
        } else {
            let mut fv = self.embed_raw(text)?;
            normalize_vec(&mut fv);
            fv
        };

        // Store in sqlite cache
        {
            let db_lock = self.db.lock().unwrap();
            if let Some(ref db) = *db_lock {
                let hex = hex_key(&key);
                let blob: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
                db.execute(
                    "INSERT OR REPLACE INTO emb_cache (query_hash, query_text, embedding) VALUES (?1, ?2, ?3)",
                    rusqlite::params![hex, text, blob],
                ).ok();
            }
        }

        self.mem_cache.lock().unwrap().put(key, v.clone());
        Ok(v)
    }

    fn embed_raw(&self, text: &str) -> Result<Vec<f32>> {
        let mut guard = self.model.lock().unwrap();
        #[allow(unused_mut)]
        if guard.is_none() {
            tracing::info!("initializing fastembed BGE-small-en-v1.5 model");
            let m = TextEmbedding::try_new(
                InitOptions::new(EmbeddingModel::BGESmallENV15)
                    .with_show_download_progress(false),
            )
            .map_err(|e| UltraError::Embed(e.to_string()))?;
            *guard = Some(m);
        }
        let model = guard.as_mut().unwrap();
        let mut result = model
            .embed(vec![text.to_string()], None)
            .map_err(|e| UltraError::Embed(e.to_string()))?;
        result
            .pop()
            .ok_or_else(|| UltraError::Embed("empty embedding result".into()))
    }

    fn hash_key(text: &str) -> [u8; 32] {
        *blake3::hash(text.as_bytes()).as_bytes()
    }
}

fn hex_key(k: &[u8; 32]) -> String {
    k.iter().map(|b| format!("{:02x}", b)).collect()
}
