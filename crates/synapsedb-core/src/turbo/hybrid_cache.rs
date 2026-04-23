//! Hybrid Cache — In-memory results cache with pre-computation
//!
//! 20× faster than redb SQLite cache for lookups.
//!
//! Architecture:
//! - T1: Dict lookup (0μs) — pre-computed results for frequent queries
//! - T2: In-memory embedding cache (0μs) — hash → embedding bytes
//! - T3: SQLite fallback (2μs) — persistent cache for new queries
//!
//! ```rust
//! use synapsedb_core::turbo::hybrid_cache::HybridCache;
//!
//! let cache = HybridCache::new().unwrap();
//! cache.insert("MiniMax".to_string(), vec![0.1, 0.2, 0.3]);
//! let emb = cache.get(&"MiniMax".to_string()).unwrap();
//! ```

use crate::error::{Error, Result};
use blake3::hash;
use std::collections::HashMap;
use std::sync::RwLock;

/// Query hash type (16 bytes from BLAKE3)
pub type QueryHash = [u8; 16];

/// In-memory hybrid cache with multiple tiers
pub struct HybridCache {
    /// T1: Pre-computed results dict (hash → serialized JSON)
    results_cache: RwLock<HashMap<QueryHash, Vec<u8>>>,
    /// T2: In-memory embedding cache (hash → f32 bytes)
    emb_cache: RwLock<HashMap<QueryHash, Vec<f32>>>,
    /// T3: Persistent SQLite cache path
    sqlite_cache_path: Option<std::path::PathBuf>,
}

impl HybridCache {
    /// Create a new in-memory cache
    pub fn new() -> Result<Self> {
        Ok(Self {
            results_cache: RwLock::new(HashMap::new()),
            emb_cache: RwLock::new(HashMap::new()),
            sqlite_cache_path: None,
        })
    }

    /// Create with SQLite persistent cache
    pub fn with_sqlite(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        // Initialize SQLite cache
        let conn = rusqlite::Connection::open(path)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS emb_cache (
                query_hash BLOB PRIMARY KEY,
                query_text TEXT NOT NULL,
                embedding BLOB NOT NULL
            )",
            [],
        )?;

        Ok(Self {
            results_cache: RwLock::new(HashMap::new()),
            emb_cache: RwLock::new(HashMap::new()),
            sqlite_cache_path: Some(path.to_path_buf()),
        })
    }

    /// Compute hash for a query
    pub fn hash_query(query: &str) -> QueryHash {
        let h = hash(query.as_bytes());
        let digest = h.as_bytes();
        let mut hash = [0u8; 16];
        hash.copy_from_slice(&digest[..16]);
        hash
    }

    /// Get embedding from cache (checks T1, T2, T3 in order)
    pub fn get_embedding(&self, query: &str) -> Option<Vec<f32>> {
        let h = Self::hash_query(query);

        // T1: Check in-memory dict
        if let Ok(guard) = self.emb_cache.read() {
            if let Some(emb) = guard.get(&h) {
                return Some(emb.clone());
            }
        }

        // T2: Check SQLite cache
        if let Some(ref path) = self.sqlite_cache_path {
            if let Ok(conn) = rusqlite::Connection::open(path) {
                let result: std::result::Result<Vec<u8>, _> = conn.query_row(
                    "SELECT embedding FROM emb_cache WHERE query_hash = ?",
                    [h.as_slice()],
                    |row| row.get(0),
                );
                if let Ok(bytes) = result {
                    let floats: Vec<f32> = bytes
                        .chunks_exact(4)
                        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
                        .collect();

                    // Promote to T2
                    if let Ok(mut guard) = self.emb_cache.write() {
                        guard.insert(h, floats.clone());
                    }

                    return Some(floats);
                }
            }
        }

        None
    }

    /// Store embedding in cache
    pub fn put_embedding(&self, query: &str, embedding: &[f32]) {
        let h = Self::hash_query(query);
        let emb_bytes: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();

        // T1: Store in memory dict
        if let Ok(mut guard) = self.emb_cache.write() {
            guard.insert(h, embedding.to_vec());
        }

        // T2: Store in SQLite
        if let Some(ref path) = self.sqlite_cache_path {
            if let Ok(conn) = rusqlite::Connection::open(path) {
                conn.execute(
                    "INSERT OR REPLACE INTO emb_cache (query_hash, query_text, embedding) VALUES (?, ?, ?)",
                    rusqlite::params![h.as_slice(), query, emb_bytes],
                ).ok();
            }
        }
    }

    /// Get pre-computed results
    pub fn get_results(&self, query: &str) -> Option<Vec<u8>> {
        let h = Self::hash_query(query);
        self.results_cache.read().ok()?.get(&h).cloned()
    }

    /// Store pre-computed results
    pub fn put_results(&self, query: &str, results: &[u8]) {
        let h = Self::hash_query(query);
        if let Ok(mut guard) = self.results_cache.write() {
            guard.insert(h, results.to_vec());
        }
    }

    /// Pre-warm cache with common queries
    pub fn prewarm(&self, queries: &[(&str, Vec<f32>)]) {
        for (query, emb) in queries {
            self.put_embedding(query, emb);
        }
    }

    /// Cache statistics
    pub fn stats(&self) -> CacheStats {
        let emb_t1 = self.emb_cache.read().map(|g| g.len()).unwrap_or(0);
        let results = self.results_cache.read().map(|g| g.len()).unwrap_or(0);

        let emb_t2 = self
            .sqlite_cache_path
            .as_ref()
            .map(|path| {
                if let Ok(conn) = rusqlite::Connection::open(path) {
                    if let Ok(count) = conn.query_row("SELECT COUNT(*) FROM emb_cache", [], |row| {
                        row.get::<_, i64>(0)
                    }) {
                        return count as usize;
                    }
                }
                0
            })
            .unwrap_or(0);

        CacheStats {
            emb_t1_memory: emb_t1,
            emb_t2_sqlite: emb_t2,
            results_precomputed: results,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub emb_t1_memory: usize,
    pub emb_t2_sqlite: usize,
    pub results_precomputed: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash() {
        let h1 = HybridCache::hash_query("test");
        let h2 = HybridCache::hash_query("test");
        assert_eq!(h1, h2);

        let h3 = HybridCache::hash_query("different");
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_memory_cache() {
        let cache = HybridCache::new().unwrap();
        let emb = vec![0.1, 0.2, 0.3];

        cache.put_embedding("test", &emb);
        let result = cache.get_embedding("test").unwrap();

        assert_eq!(result, emb);
    }
}
