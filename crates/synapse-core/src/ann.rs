//! PR-A1-wire: optional usearch ANN index integrated into `Store`.
//!
//! Feature-gated under `ann-usearch` (default OFF per SPEC §3). When enabled,
//! the `Store` carries an in-memory usearch HNSW index alongside `docs_vec`
//! and persists a sidecar file next to `brain.db`. On `open`, we try to load
//! the sidecar; if it is missing or corrupt, we rebuild from `docs_vec` rows.
//!
//! Writes: `put_inner` / `put_batch` MUST call `Ann::insert` after the SQL
//! transaction commits. Reads: `Store::search_vec` tries the ANN fast path
//! first and falls back to sqlite-vec on any error.
//!
//! Crash semantics: because the sidecar is rebuildable from `docs_vec`, a
//! SIGKILL between SQL commit and sidecar save is recoverable — next `open`
//! rebuilds. Data loss is bounded to the last uncommitted SQL batch, same
//! as baseline Synapse.

#![allow(clippy::type_complexity)]
#![cfg(feature = "ann-usearch")]

use crate::error::{Error, Result};
use parking_lot::RwLock;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use synapse_ann::AnnIndex;
use synapse_ann::usearch_backend::{UsearchIndex, default_sidecar_path};

/// Thread-safe wrapper around a usearch index + its sidecar path.
pub struct Ann {
    inner: Arc<RwLock<UsearchIndex>>,
    sidecar: PathBuf,
    dim: usize,
}

impl Ann {
    /// Derive the sidecar path next to `db_path` (i.e. `brain.db.usearch`)
    /// and return the conventional location used by `Store`.
    pub fn sidecar_for(db_path: &Path) -> PathBuf {
        default_sidecar_path(db_path)
    }

    /// Open or rebuild the ANN index.
    ///
    /// Resolution order:
    /// 1. Try to load `<db>.usearch`.
    /// 2. If missing or corrupt, return an empty index — the caller is
    ///    expected to `rebuild_from_rows` from `docs_vec`.
    pub fn open_or_empty(sidecar: PathBuf, dim: usize, expected_capacity: usize) -> Result<Self> {
        let inner = match UsearchIndex::try_load_or_none(&sidecar, dim) {
            Ok(Some(i)) => i,
            Ok(None) => UsearchIndex::new(dim, expected_capacity)
                .map_err(|e| Error::Other(format!("usearch new: {e}")))?,
            Err(e) => return Err(Error::Other(format!("usearch load: {e}"))),
        };
        Ok(Self {
            inner: Arc::new(RwLock::new(inner)),
            sidecar,
            dim,
        })
    }

    /// Build an empty index from scratch (no load attempt). Used after a
    /// detected version mismatch or corruption, before `rebuild_from_rows`.
    pub fn fresh(sidecar: PathBuf, dim: usize, expected_capacity: usize) -> Result<Self> {
        let inner = UsearchIndex::new(dim, expected_capacity)
            .map_err(|e| Error::Other(format!("usearch new: {e}")))?;
        Ok(Self {
            inner: Arc::new(RwLock::new(inner)),
            sidecar,
            dim,
        })
    }

    /// Current number of vectors indexed.
    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Pre-reserve capacity for `additional` entries. Call before bulk
    /// `insert_or_skip` to prevent "Reserve capacity ahead of insertions!" crash
    /// when the loaded sidecar didn't have headroom for new tail entries.
    pub fn ensure_capacity_for_tail(&self, additional: usize) -> Result<()> {
        self.inner
            .write()
            .ensure_capacity(additional)
            .map_err(|e| Error::Other(format!("ann ensure_capacity: {e}")))
    }

    /// Insert `(id, vec)`. Called from `Store::put_inner` / `put_batch` after
    /// the SQL transaction successfully commits.
    pub fn insert(&self, id: i64, vec: &[f32]) -> Result<()> {
        self.inner
            .write()
            .insert(id as u64, vec)
            .map_err(|e| Error::Other(format!("usearch insert: {e}")))
    }

    /// Insert if not already present. Returns true when a new vector was added.
    /// Used during sidecar tail-rebuild after load when concurrent puts may
    /// have added rows after last persist. Duplicate errors are skipped.
    pub fn insert_or_skip(&self, id: i64, vec: &[f32]) -> Result<bool> {
        let mut g = self.inner.write();
        // Ensure capacity before insert — prevents "Reserve capacity ahead of insertions!" crash.
        if let Err(e) = g.ensure_capacity(1) {
            let msg = format!("{e:?}");
            if !msg.contains("Duplicate") {
                tracing::warn!("insert_or_skip reserve failed: {msg}");
            }
        }
        match g.insert(id as u64, vec) {
            Ok(()) => Ok(true),
            Err(e) => {
                let msg = format!("{e:?}");
                if msg.contains("Duplicate") || msg.contains("already exists") {
                    Ok(false)
                } else {
                    Err(Error::Other(format!("usearch insert_or_skip: {e}")))
                }
            }
        }
    }

    /// Remove `id`. Idempotent.
    pub fn remove(&self, id: i64) -> Result<usize> {
        self.inner
            .write()
            .remove(id as u64)
            .map_err(|e| Error::Other(format!("usearch remove: {e}")))
    }

    /// Current runtime ef_search (expansion_search).
    pub fn expansion_search(&self) -> usize {
        self.inner.read().expansion_search()
    }

    /// kNN search with a temporary ef boost (higher recall, higher latency).
    /// ef is clamped to [k, 4096]. Returns `(id, distance)` pairs.
    pub fn search_with_ef(&self, query: &[f32], k: usize, ef: usize) -> Result<Vec<(i64, f32)>> {
        if query.len() != self.dim {
            return Err(Error::DimMismatch {
                expected: self.dim,
                got: query.len(),
            });
        }
        let g = self.inner.read();
        let out = g
            .search_with_ef(query, k, ef)
            .map_err(|e| Error::Other(format!("usearch search_with_ef: {e}")))?;
        Ok(out.into_iter().map(|(id, d)| (id as i64, d)).collect())
    }

    /// kNN search. Returns `(id, distance)` pairs. Callers join against
    /// `docs` for full hit records.
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<(i64, f32)>> {
        if query.len() != self.dim {
            return Err(Error::DimMismatch {
                expected: self.dim,
                got: query.len(),
            });
        }
        let g = self.inner.read();
        let out = g
            .search(query, k)
            .map_err(|e| Error::Other(format!("usearch search: {e}")))?;
        Ok(out.into_iter().map(|(id, d)| (id as i64, d)).collect())
    }

    /// Flush the sidecar to disk atomically.
    pub fn save(&self) -> Result<()> {
        self.inner
            .read()
            .save(&self.sidecar)
            .map_err(|e| Error::Other(format!("usearch save: {e}")))
    }

    /// Rebuild the index from provided `(id, vec)` pairs. Used when the
    /// sidecar is missing or corrupt. Caller iterates `docs_vec`.
    pub fn rebuild_from_rows<I>(&self, rows: I) -> Result<usize>
    where
        I: IntoIterator<Item = (i64, Vec<f32>)>,
    {
        let rows: Vec<(i64, Vec<f32>)> = rows.into_iter().collect();
        let mut g = self.inner.write();
        g.ensure_capacity(rows.len())
            .map_err(|e| Error::Other(format!("usearch rebuild reserve: {e}")))?;
        let mut n = 0usize;
        for (id, v) in rows {
            match g.insert(id as u64, &v) {
                Ok(()) => n += 1,
                Err(e) => {
                    let msg = format!("{e:?}");
                    if msg.contains("Duplicate") || msg.contains("already exists") {
                        tracing::warn!("usearch rebuild skipped duplicate id={id}");
                    } else {
                        return Err(Error::Other(format!("usearch rebuild insert: {e}")));
                    }
                }
            }
        }
        Ok(n)
    }
}
