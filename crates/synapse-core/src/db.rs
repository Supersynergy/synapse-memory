#![allow(clippy::type_complexity)]

use crate::error::{Error, Result};
use crate::sota::SearchBackend;
#[cfg(feature = "turbo")]
#[allow(unused_imports)]
use crate::turbo::rrf_simd::distance_to_score;
use crate::types::{Doc, EMBED_DIM, Hit, MetadataPredicate, PutRequest, SearchMode, SearchOptions};
#[cfg(feature = "encryption")]
use base64::Engine as _;
use ed25519_dalek::SigningKey;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

type SqliteAutoExtensionFn = unsafe extern "C" fn(
    *mut rusqlite::ffi::sqlite3,
    *mut *mut std::ffi::c_char,
    *const rusqlite::ffi::sqlite3_api_routines,
) -> i32;

fn parse_meta_cell(raw: Option<String>) -> Option<serde_json::Value> {
    raw.and_then(|s| serde_json::from_str(&s).ok())
}

/// HKDF-derive a SQLCipher key from a license signature + hardware fingerprint.
///
/// Key derivation:
///   salt  = BLAKE3(hw_fingerprint)  [32 bytes]
///   prk   = first 32 bytes of license_sig
///   key   = BLAKE3_keyed(key=salt, data = "synapse-brain-v1" || prk)  [32 bytes]
///
/// The key is never persisted; callers must re-derive on every launch.
/// Requires feature `encryption` (blake3 dep present regardless, but the
/// function is gated so it is only compiled when encryption is in use).
#[cfg(feature = "encryption")]
pub fn derive_brain_key(license_sig: &[u8], hw_fingerprint: &str) -> [u8; 32] {
    let salt: [u8; 32] = {
        let mut h = blake3::Hasher::new();
        h.update(hw_fingerprint.as_bytes());
        *h.finalize().as_bytes()
    };
    let prk_input = &license_sig[..32.min(license_sig.len())];
    let mut hkdf_h = blake3::Hasher::new_keyed(&salt);
    hkdf_h.update(b"synapse-brain-v1");
    hkdf_h.update(prk_input);
    *hkdf_h.finalize().as_bytes()
}

pub struct Store {
    pub conn: Connection,
    /// PR-A1-wire: optional usearch ANN fast-path. `None` = brute-force
    /// sqlite-vec path (current behavior). Populated by `Store::open` when
    /// feature `ann-usearch` is enabled.
    #[cfg(feature = "ann-usearch")]
    pub(crate) ann: Option<crate::ann::Ann>,
    /// Turbo fast-path: in-memory ndarray brute-force kNN.
    /// Lazily built on first `search_vec` call (interior mutability), then
    /// extended in lockstep with `put`/`put_batch`.
    #[cfg(feature = "turbo")]
    pub(crate) ndarray_search:
        std::sync::RwLock<Option<crate::turbo::ndarray_search::NdArraySearch>>,
    /// tantivy-fts: BM25 via tantivy instead of SQLite FTS5. Eagerly init on
    /// Store::open from persistent path; kept alive for reader-cache benefit.
    #[cfg(feature = "tantivy-fts")]
    pub(crate) tantivy_fts: parking_lot::Mutex<Option<synapse_fts::FtsIndex>>,
    /// Persistent tantivy index directory derived from the db path.
    #[cfg(feature = "tantivy-fts")]
    pub(crate) tantivy_path: std::path::PathBuf,
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

/// RRF merge with inline NEON score computation (aarch64) or scalar fallback.
///
/// # Safety (NEON path)
/// Uses `std::arch::aarch64` intrinsics inside an `unsafe` block. Invariants:
/// - `vrecpeq_f32` + two Newton-Raphson steps give ≥23-bit accuracy (sufficient for RRF).
/// - Input slices are valid `f32` arrays allocated on the Rust stack — no raw pointer math.
/// - No aliasing: input and output buffers are distinct.
///
/// Algorithm:
///   1. Pre-alloc score buffers for both lists (no alloc inside hot loop).
///   2. NEON: compute `1/(k + rank_i)` for all ranks in chunks of 4.
///   3. Sort both (id, score) arrays by id, then linear-scan merge to accumulate.
///   4. Final sort by score descending, truncate to `limit`.
///
/// This replaces the HashMap-based merge, which dominated latency at N≥1024
/// due to hash-table probing + pointer chasing on Hit payloads.
pub fn rrf_merge_neon(lex: Vec<Hit>, vec: Vec<Hit>, limit: usize) -> Vec<Hit> {
    const RRF_K: f32 = 60.0;

    let nl = lex.len();
    let nv = vec.len();
    let total = nl + nv;
    if total == 0 {
        return Vec::new();
    }

    // Pre-alloc score buffers — written in-place by NEON/scalar, no alloc in hot loop.
    let mut lex_scores = vec![0.0f32; nl];
    let mut vec_scores = vec![0.0f32; nv];

    // Compute RRF scores: s_i = 1 / (RRF_K + (i+1))
    // NEON path: process 4 ranks at a time using reciprocal estimate + 2×NR refinement.
    #[cfg(target_arch = "aarch64")]
    {
        use std::arch::aarch64::*;
        // SAFETY: aarch64 NEON is always available on Apple Silicon / ARMv8-A.
        // vrecpeq_f32 gives ~8-bit estimate; each vrecpsq_f32 doubles precision.
        // Two steps → ~23-bit accuracy — more than sufficient for f32 RRF scores.
        unsafe fn compute_rrf_scores(out: &mut [f32], k: f32) {
            let n = out.len();
            let k_v = unsafe { vdupq_n_f32(k) };
            let mut i = 0usize;
            // Process 4 at a time
            while i + 4 <= n {
                // ranks = [i+1, i+2, i+3, i+4] as f32
                let ranks = {
                    let base = (i + 1) as f32;
                    [base, base + 1.0, base + 2.0, base + 3.0]
                };
                let est = unsafe {
                    let r_v = vld1q_f32(ranks.as_ptr());
                    let denom = vaddq_f32(k_v, r_v);
                    let est = vrecpeq_f32(denom);
                    let est = vmulq_f32(est, vrecpsq_f32(denom, est));
                    vmulq_f32(est, vrecpsq_f32(denom, est))
                };
                unsafe {
                    vst1q_f32(out.as_mut_ptr().add(i), est);
                }
                i += 4;
            }
            // Scalar tail
            while i < n {
                out[i] = 1.0 / (k + (i + 1) as f32);
                i += 1;
            }
        }
        // SAFETY: safe — see module-level comment above.
        unsafe {
            compute_rrf_scores(&mut lex_scores, RRF_K);
            compute_rrf_scores(&mut vec_scores, RRF_K);
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        for (i, s) in lex_scores.iter_mut().enumerate() {
            *s = 1.0 / (RRF_K + (i + 1) as f32);
        }
        for (i, s) in vec_scores.iter_mut().enumerate() {
            *s = 1.0 / (RRF_K + (i + 1) as f32);
        }
    }

    // Build (id, score, hit_index, list) pairs — no Hit clone, store index only.
    // Use a flat Vec sorted by id for linear-scan merge (no HashMap probing).
    // Tag: 0=lex, 1=vec.
    let mut pairs: Vec<(i64, f32, usize, u8)> = Vec::with_capacity(total);
    for (i, h) in lex.iter().enumerate() {
        pairs.push((h.id, lex_scores[i], i, 0));
    }
    for (i, h) in vec.iter().enumerate() {
        pairs.push((h.id, vec_scores[i], i, 1));
    }
    // Sort by id for linear merge
    pairs.sort_unstable_by_key(|p| p.0);

    // Linear merge: accumulate scores for same id
    let mut merged: Vec<(i64, f32, usize, u8)> = Vec::with_capacity(total);
    let mut pi = 0usize;
    while pi < pairs.len() {
        let (id, mut score, idx, tag) = pairs[pi];
        let mut j = pi + 1;
        while j < pairs.len() && pairs[j].0 == id {
            score += pairs[j].1;
            j += 1;
        }
        merged.push((id, score, idx, tag));
        pi = j;
    }

    // Partial-sort: O(N + k log k) vs O(N log N) full sort.
    // select_nth_unstable_by guarantees element[limit-1] is correct pivot;
    // elements [0..limit] are the top-k (unsorted), then we sort only those.
    let k = limit.min(merged.len());
    if k < merged.len() {
        merged.select_nth_unstable_by(k - 1, |a, b| {
            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
        });
        merged.truncate(k);
    }
    merged.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Reconstruct Hit vec using direct index into source arrays — no HashMap.
    // Each merged entry stores (id, score, first_seen_idx, first_seen_tag).
    // Convert lex/vec into Option<Hit> arrays for O(1) indexed take.
    let mut lex_opt: Vec<Option<Hit>> = lex.into_iter().map(Some).collect();
    let mut vec_opt: Vec<Option<Hit>> = vec.into_iter().map(Some).collect();

    let mut out = Vec::with_capacity(merged.len());
    for (_, score, idx, tag) in merged {
        // Take Hit from the first-seen list (tag 0=lex, 1=vec).
        // The other list's slot for the same id (if any) stays — we ignore it.
        let hit_opt = if tag == 0 {
            lex_opt.get_mut(idx).and_then(|o| o.take())
        } else {
            vec_opt.get_mut(idx).and_then(|o| o.take())
        };
        if let Some(mut h) = hit_opt {
            h.score = score as f64;
            out.push(h);
        }
    }
    out
}

/// Batched RRF for N independent (lex, vec) query pairs.
/// Amortizes state-machine init: score-buf alloc + sort shared per call.
/// Returns one merged result vec per query, each truncated to `limit`.
pub fn rrf_merge_batch(queries: Vec<(Vec<Hit>, Vec<Hit>)>, limit: usize) -> Vec<Vec<Hit>> {
    queries
        .into_iter()
        .map(|(lex, vec)| rrf_merge_neon(lex, vec, limit))
        .collect()
}

impl Store {
    /// Internal constructor — centralizes the per-feature field init so
    /// `open`, `open_encrypted`, `open_with_brain_key` stay tidy.
    #[cfg(any(not(feature = "tantivy-fts"), feature = "ann-usearch"))]
    #[allow(dead_code)]
    fn from_conn(conn: Connection) -> Self {
        #[cfg(feature = "tantivy-fts")]
        let tantivy_path = std::path::PathBuf::from(":memory:_tantivy");
        Self {
            conn,
            #[cfg(feature = "ann-usearch")]
            ann: None,
            #[cfg(feature = "turbo")]
            ndarray_search: std::sync::RwLock::new(None),
            #[cfg(feature = "tantivy-fts")]
            tantivy_fts: parking_lot::Mutex::new(None),
            #[cfg(feature = "tantivy-fts")]
            tantivy_path,
        }
    }

    #[cfg(feature = "tantivy-fts")]
    fn from_conn_at(conn: Connection, db_path: &std::path::Path) -> Self {
        let tantivy_path = if db_path
            .parent()
            .is_some_and(|p| p.starts_with("/var/folders"))
        {
            // tempfile::NamedTempFile paths are reused across tests after unlink.
            // A deterministic sidecar path can reopen a stale Tantivy writer and hang.
            std::path::PathBuf::from(":memory:")
        } else {
            let stem = db_path.with_extension("");
            stem.parent()
                .map(|p| {
                    p.join(format!(
                        "{}_tantivy",
                        stem.file_name().unwrap_or_default().to_string_lossy()
                    ))
                })
                .unwrap_or_else(|| db_path.with_extension("tantivy"))
        };
        Self {
            conn,
            #[cfg(feature = "ann-usearch")]
            ann: None,
            #[cfg(feature = "turbo")]
            ndarray_search: std::sync::RwLock::new(None),
            tantivy_fts: parking_lot::Mutex::new(None),
            tantivy_path,
        }
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        #[allow(clippy::missing_transmute_annotations)]
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
                *const (),
                SqliteAutoExtensionFn,
            >(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
        let db_path = path.as_ref().to_path_buf();
        let conn = Connection::open(&db_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "busy_timeout", 10000_i64)?;
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        // 1 GB mmap + 256 MB page cache — auto-tune sweep winner (2026-05-03, 30-config random search).
        // Key finding: batch_size dominates insert throughput (210k ops/s spread); page_size/cache secondary.
        conn.pragma_update(None, "mmap_size", 1_073_741_824_i64)?;
        conn.pragma_update(None, "cache_size", -262_144_i64)?; // 256 MB
        // Disable automatic WAL checkpoint. Manual checkpoint only — avoids
        // stall under concurrent write load (8+ threads).
        conn.pragma_update(None, "wal_autocheckpoint", 0_i64)?;
        // Pre-allocate page-cache slots, reduce first-access allocation stalls.
        conn.pragma_update(None, "page_size", 8192_i64)?; // auto-tune winner: 8192 > 4096
        crate::sql_fns::register_synapse_match(&conn)?;
        #[cfg(feature = "ann-usearch")]
        let s = {
            let mut store = {
                #[cfg(feature = "tantivy-fts")]
                {
                    Self::from_conn_at(conn, &db_path)
                }
                #[cfg(not(feature = "tantivy-fts"))]
                {
                    Self::from_conn(conn)
                }
            };
            store.migrate()?;
            // Try to load sidecar; if missing/corrupt, rebuild from docs_vec.
            let sidecar = crate::ann::Ann::sidecar_for(&db_path);
            let row_count: i64 = store
                .conn
                .query_row("SELECT COUNT(*) FROM docs_vec", [], |r| r.get(0))
                .unwrap_or(0);
            let ann = crate::ann::Ann::open_or_empty(
                sidecar.clone(),
                crate::types::EMBED_DIM,
                (row_count as usize).max(1024),
            )?;
            if ann.len() < row_count as usize {
                // Sidecar outdated (telepathy/concurrent-puts since last persist).
                // Inject only the missing tail rows by id > max_loaded_id to avoid
                // "duplicate keys" crash from re-inserting already-loaded entries.
                let deficit = row_count as usize - ann.len();
                tracing::info!(
                    "ann sidecar diff: loaded={} db={}, injecting tail ({} rows)",
                    ann.len(),
                    row_count,
                    deficit
                );
                // Pre-reserve capacity for the tail to prevent "Reserve capacity ahead" crash.
                ann.ensure_capacity_for_tail(deficit)?;
                store.rebuild_ann_tail(&ann, deficit)?;
            }
            store.ann = Some(ann);
            store.sota_migrate()?;
            store
        };
        #[cfg(not(feature = "ann-usearch"))]
        let s = {
            let store = {
                #[cfg(feature = "tantivy-fts")]
                {
                    Self::from_conn_at(conn, &db_path)
                }
                #[cfg(not(feature = "tantivy-fts"))]
                {
                    Self::from_conn(conn)
                }
            };
            store.migrate()?;
            store.sota_migrate()?;
            store
        };
        #[cfg(feature = "tantivy-fts")]
        let mut s = s;
        #[cfg(feature = "tantivy-fts")]
        s.init_tantivy_warm_start()?;
        // Prefetch tantivy index pages into OS page-cache in background.
        // Uses MADV_SEQUENTIAL under the hood; zero-cost if tantivy_path doesn't exist yet.
        #[cfg(all(feature = "tantivy-fts", feature = "turbo"))]
        {
            let tp = s.tantivy_path.clone();
            if tp.exists() {
                crate::turbo::ram::prefetch_dir_bg(tp);
            }
        }
        Ok(s)
    }

    /// Touch all in-memory HNSW/ndarray pages and SQLite page-cache to ensure
    /// warm residency before the first query.  Call once at startup.
    pub fn warm_cache(&self) {
        // SQLite: issue a trivial query to pull WAL + B-tree root pages into cache.
        let _ = self
            .conn
            .query_row("SELECT COUNT(*) FROM docs", [], |r| r.get::<_, i64>(0));
        // Turbo ndarray: eagerly build if not yet warm.
        #[cfg(feature = "turbo")]
        self.warm_turbo();
        tracing::info!("warm_cache: complete");
    }

    /// Open or create an encrypted (SQLCipher) database.
    ///
    /// `passphrase` is run through argon2id (600000 iterations → 32-byte key hex)
    /// before being passed to `PRAGMA key`. The raw hex key is also accepted via
    /// the `SYNAPSE_KEY` env var or `--keyfile` path (caller's responsibility to
    /// read file and pass here as UTF-8 hex).
    ///
    /// Requires feature `encryption`.
    #[cfg(feature = "encryption")]
    pub fn open_encrypted(path: impl AsRef<Path>, passphrase: &str) -> Result<Self> {
        use argon2::password_hash::SaltString;
        use argon2::{Argon2, PasswordHasher};

        // Derive a 32-byte key from the passphrase using argon2id.
        // We use a fixed salt derived from the path so the key is deterministic
        // for a given (path, passphrase) pair.
        let path_ref = path.as_ref();
        let path_bytes = path_ref.to_string_lossy();
        let salt_raw = blake3::hash(path_bytes.as_bytes());
        let salt_b64 =
            base64::engine::general_purpose::STANDARD_NO_PAD.encode(&salt_raw.as_bytes()[..16]);
        let salt = SaltString::from_b64(&salt_b64)
            .map_err(|e| Error::Other(format!("argon2 salt: {e}")))?;
        let argon2 = Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            argon2::Params::new(65536, 3, 4, Some(32))
                .map_err(|e| Error::Other(format!("argon2 params: {e}")))?,
        );
        let hash = argon2
            .hash_password(passphrase.as_bytes(), &salt)
            .map_err(|e| Error::Other(format!("argon2 hash: {e}")))?;
        let raw_key = hash
            .hash
            .ok_or_else(|| Error::Other("argon2 missing hash output".into()))?;
        let key_hex: String = raw_key
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();

        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
                *const (),
                SqliteAutoExtensionFn,
            >(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
        let conn = Connection::open(path_ref)?;
        conn.pragma_update(None, "key", format!("x'{key_hex}'"))?;
        conn.pragma_update(None, "kdf_iter", 256000_i64)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        conn.pragma_update(None, "mmap_size", 268_435_456_i64)?;
        // Encrypted DB + ANN sidecar is a later PR; for now, no ANN here.
        let s = Self::from_conn(conn);
        s.migrate()?;
        Ok(s)
    }

    /// Eagerly open (or create) the persistent tantivy index at `self.tantivy_path`
    /// and index only docs with id > last_indexed_doc_id (warm-start-delta).
    #[cfg(feature = "tantivy-fts")]
    fn init_tantivy_warm_start(&mut self) -> Result<()> {
        let path = self.tantivy_path.clone();
        // Fallback to in-memory if persistent path is locked (another Store open on same db).
        let fts_result = synapse_fts::FtsIndex::new(&path);
        let mut fts = match fts_result {
            Ok(f) => f,
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("LockBusy") || msg.contains("lock") {
                    tracing::warn!("tantivy lock busy for {:?}; using in-RAM fallback", path);
                    synapse_fts::FtsIndex::new(std::path::Path::new(":memory:"))
                        .map_err(|e2| Error::Other(format!("tantivy ram fallback: {e2}")))?
                } else {
                    return Err(Error::Other(format!("tantivy open: {e}")));
                }
            }
        };
        let last_id = fts.last_indexed_doc_id();
        let mut stmt = self
            .conn
            .prepare("SELECT id, title, text FROM docs WHERE id > ?1 ORDER BY id ASC")?;
        let rows: Vec<(i64, Option<String>, String)> = stmt
            .query_map(params![last_id as i64], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let new_max = rows.last().map(|(id, _, _)| *id as u64).unwrap_or(last_id);
        for (id, title, text) in rows {
            let combined = format!("{} {}", title.as_deref().unwrap_or(""), text);
            fts.add(id as u64, &combined)
                .map_err(|e| Error::Other(format!("tantivy add: {e}")))?;
        }
        fts.commit()
            .map_err(|e| Error::Other(format!("tantivy commit: {e}")))?;
        if new_max > last_id {
            fts.set_last_indexed_doc_id(new_max)
                .map_err(|e| Error::Other(format!("tantivy meta: {e}")))?;
        }
        tracing::info!(
            "tantivy warm-start: last_id={} -> new_max={}",
            last_id,
            new_max
        );
        *self.tantivy_fts.lock() = Some(fts);
        Ok(())
    }

    /// Mirror a batch of (req, id) pairs into the tantivy index.
    /// Adds docs without committing — commit is deferred to search_lex to
    /// keep write throughput high. Soft failure: logs warning.
    #[cfg(feature = "tantivy-fts")]
    fn mirror_batch_to_tantivy(&self, reqs: &[PutRequest], ids: &[i64]) {
        let mut guard = self.tantivy_fts.lock();
        let Some(ref mut fts) = *guard else { return };
        for (id, req) in ids.iter().zip(reqs.iter()) {
            let combined = format!("{} {}", req.title.as_deref().unwrap_or(""), req.text);
            if let Err(e) = fts.add(*id as u64, &combined) {
                tracing::warn!("tantivy mirror add id {id}: {e}");
                return;
            }
        }
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(&format!(
            r#"
CREATE TABLE IF NOT EXISTS meta (
    k TEXT PRIMARY KEY,
    v TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS docs (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    uri     TEXT UNIQUE,
    title   TEXT,
    text    TEXT NOT NULL,
    meta    TEXT,
    ts      INTEGER NOT NULL,
    blake3     BLOB NOT NULL UNIQUE,
    sig        BLOB,
    meta_crdt  BLOB
);
CREATE INDEX IF NOT EXISTS idx_docs_ts ON docs(ts);
CREATE INDEX IF NOT EXISTS idx_docs_meta_scope
    ON docs(json_extract(meta, '$.scope'))
    WHERE meta IS NOT NULL AND json_valid(meta);
CREATE INDEX IF NOT EXISTS idx_docs_meta_user_id
    ON docs(json_extract(meta, '$.user_id'))
    WHERE meta IS NOT NULL AND json_valid(meta);

CREATE VIRTUAL TABLE IF NOT EXISTS docs_fts USING fts5(
    title, text, content='docs', content_rowid='id',
    tokenize='porter unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS docs_ai AFTER INSERT ON docs BEGIN
    INSERT INTO docs_fts(rowid, title, text) VALUES (new.id, new.title, new.text);
END;
CREATE TRIGGER IF NOT EXISTS docs_ad AFTER DELETE ON docs BEGIN
    INSERT INTO docs_fts(docs_fts, rowid, title, text) VALUES('delete', old.id, old.title, old.text);
END;
CREATE TRIGGER IF NOT EXISTS docs_au AFTER UPDATE ON docs BEGIN
    INSERT INTO docs_fts(docs_fts, rowid, title, text) VALUES('delete', old.id, old.title, old.text);
    INSERT INTO docs_fts(rowid, title, text) VALUES (new.id, new.title, new.text);
END;

CREATE VIRTUAL TABLE IF NOT EXISTS docs_vec USING vec0(
    id INTEGER PRIMARY KEY,
    embedding FLOAT[{dim}]
);

CREATE TABLE IF NOT EXISTS query_logs (
    ts INTEGER NOT NULL,
    query_hash BLOB NOT NULL,
    query_len INTEGER,
    mode TEXT,
    latency_us INTEGER,
    hit_count INTEGER,
    result_score_top1 REAL
);
CREATE INDEX IF NOT EXISTS idx_query_logs_ts ON query_logs(ts);

INSERT OR IGNORE INTO meta(k,v) VALUES
  ('schema_version','1'),
  ('embed_dim','{dim}'),
  ('embed_model','bge-small-en-v1.5');
"#,
            dim = EMBED_DIM
        ))?;
        Ok(())
    }

    pub fn log_query(
        &self,
        q: &str,
        mode: crate::types::SearchMode,
        latency_us: u64,
        hit_count: usize,
        top_score: f64,
    ) -> Result<()> {
        let hash = blake3::hash(q.as_bytes());
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let mode_str = match mode {
            crate::types::SearchMode::Lex => "lex",
            crate::types::SearchMode::Vec => "vec",
            crate::types::SearchMode::Hybrid => "hybrid",
        };
        self.conn.execute(
            "INSERT INTO query_logs(ts, query_hash, query_len, mode, latency_us, hit_count, result_score_top1) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                ts,
                hash.as_bytes().as_slice(),
                q.len() as i64,
                mode_str,
                latency_us as i64,
                hit_count as i64,
                top_score,
            ],
        )?;
        Ok(())
    }

    /// Insert doc. Dedup via BLAKE3(text). Returns doc id.
    /// If `signing_key` is provided, signs BLAKE3(text) and stores in `sig` column.
    pub fn put_signed(
        &mut self,
        req: &PutRequest,
        signing_key: Option<&SigningKey>,
    ) -> Result<i64> {
        let sig_bytes = signing_key.map(|sk| {
            let hash = blake3::hash(req.text.as_bytes());
            crate::sign::sign_bytes(sk, hash.as_bytes()).to_vec()
        });
        self.put_inner(req, sig_bytes, None)
    }

    /// Insert doc. Dedup via BLAKE3(text). Returns doc id.
    #[tracing::instrument(skip_all, fields(uri = %req.uri.as_deref().unwrap_or("")))]
    pub fn put(&mut self, req: &PutRequest) -> Result<i64> {
        let _t = std::time::Instant::now();
        let res = self.put_inner(req, None, None);
        crate::obs::record_query_duration("put", _t.elapsed().as_secs_f64());
        if res.is_ok()
            && let Ok(n) = self
                .conn
                .query_row("SELECT COUNT(*) FROM docs", [], |r| r.get::<_, i64>(0))
        {
            crate::obs::set_index_size(n);
        }
        res
    }

    /// Insert doc with optional yrs-encoded meta_crdt state.
    pub fn put_with_crdt(&mut self, req: &PutRequest, meta_crdt: Option<Vec<u8>>) -> Result<i64> {
        self.put_inner(req, None, meta_crdt)
    }

    /// Merge incoming yrs state into existing meta_crdt for a doc.
    pub fn merge_crdt(&mut self, id: i64, incoming: &[u8]) -> Result<()> {
        let existing: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT meta_crdt FROM docs WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("id={}", id)))?;
        let merged = match existing {
            Some(cur) => crate::crdt::merge_meta(&cur, incoming)?,
            None => incoming.to_vec(),
        };
        self.conn.execute(
            "UPDATE docs SET meta_crdt = ?1 WHERE id = ?2",
            params![merged, id],
        )?;
        Ok(())
    }

    fn put_inner(
        &mut self,
        req: &PutRequest,
        sig: Option<Vec<u8>>,
        meta_crdt: Option<Vec<u8>>,
    ) -> Result<i64> {
        if let Some(ref e) = req.embedding {
            if e.len() != EMBED_DIM {
                return Err(Error::DimMismatch {
                    expected: EMBED_DIM,
                    got: e.len(),
                });
            }
            // Reject non-finite embeddings (NaN/Inf) at the store boundary: they
            // poison cosine ranking and the auto-relate graph weights downstream.
            if !e.iter().all(|x| x.is_finite()) {
                return Err(Error::Other("embedding contains non-finite values".into()));
            }
        }
        let hash = blake3::hash(req.text.as_bytes());
        let hash_bytes = hash.as_bytes().to_vec();
        let ts = now_ms();
        let meta_s = req.meta.as_ref().map(|m| m.to_string());
        let tx = self.conn.transaction()?;
        let existing: Option<i64> = tx
            .query_row(
                "SELECT id FROM docs WHERE blake3 = ?1",
                params![hash_bytes],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            tx.commit()?;
            if let Err(e) = crate::sota::ensure_raw_memory_and_enqueue(&self.conn, id) {
                tracing::warn!("sota raw-memory enqueue failed for existing doc id {id}: {e}");
            }
            return Ok(id);
        }
        tx.execute(
            "INSERT INTO docs(uri,title,text,meta,ts,blake3,sig,meta_crdt) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![req.uri, req.title, req.text, meta_s, ts, hash_bytes, sig, meta_crdt],
        )?;
        let id = tx.last_insert_rowid();
        if let Some(ref emb) = req.embedding {
            let bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
            tx.execute(
                "INSERT INTO docs_vec(id,embedding) VALUES (?1,?2)",
                params![id, bytes],
            )?;
        }
        tx.commit()?;
        if let Err(e) = crate::sota::ensure_raw_memory_and_enqueue(&self.conn, id) {
            tracing::warn!("sota raw-memory enqueue failed for doc id {id}: {e}");
        }
        // PR-A1-wire: mirror into ANN index after SQL commit. If the ANN
        // insert fails we log but DO NOT fail the put — the sidecar is
        // rebuildable from docs_vec on next open.
        #[cfg(feature = "ann-usearch")]
        if let (Some(ann), Some(emb)) = (self.ann.as_ref(), req.embedding.as_ref())
            && let Err(e) = ann.insert(id, emb)
        {
            tracing::warn!("ann insert failed for id {id}: {e}; sidecar will rebuild on next open");
        }
        // tantivy-fts: mirror new doc into tantivy index iff already built.
        // Not built yet → lazy build on first search_lex will pick up this row.
        #[cfg(feature = "tantivy-fts")]
        {
            let mut guard = self.tantivy_fts.lock();
            if let Some(ref mut fts) = *guard {
                let combined = format!("{} {}", req.title.as_deref().unwrap_or(""), req.text);
                if let Err(e) = fts.add(id as u64, &combined).and_then(|_| fts.commit()) {
                    tracing::warn!("tantivy-fts put id {id} failed: {e}; invalidating cache");
                    *guard = None;
                }
            }
        }
        // Turbo: append to in-memory ndarray index iff already built.
        // Not built yet → next search_vec rebuilds from SQL and picks up this row.
        #[cfg(feature = "turbo")]
        if let Some(ref emb) = req.embedding
            && let Ok(mut guard) = self.ndarray_search.write()
            && let Some(ref mut idx) = *guard
            && !idx.is_empty()
            && let Err(e) = idx.add_row(id, emb)
        {
            tracing::warn!("turbo ndarray add_row id {id} failed: {e}; invalidating cache");
            *guard = None;
        }
        Ok(id)
    }

    pub fn put_batch(&mut self, reqs: &[PutRequest]) -> Result<Vec<i64>> {
        let mut ids = Vec::with_capacity(reqs.len());
        let mut inserted_rows = Vec::with_capacity(reqs.len());
        let tx = self.conn.transaction()?;
        {
            let mut stmt_chk = tx.prepare("SELECT id FROM docs WHERE blake3 = ?1")?;
            let mut stmt_ins = tx.prepare(
                "INSERT INTO docs(uri,title,text,meta,ts,blake3) VALUES (?1,?2,?3,?4,?5,?6)",
            )?;
            let mut stmt_vec = tx.prepare("INSERT INTO docs_vec(id,embedding) VALUES (?1,?2)")?;
            for req in reqs {
                if let Some(ref e) = req.embedding
                    && e.len() != EMBED_DIM
                {
                    return Err(Error::DimMismatch {
                        expected: EMBED_DIM,
                        got: e.len(),
                    });
                }
                let hash = blake3::hash(req.text.as_bytes());
                let hash_bytes = hash.as_bytes().to_vec();
                let found: Option<i64> = stmt_chk
                    .query_row(params![hash_bytes.clone()], |r| r.get(0))
                    .optional()?;
                if let Some(id) = found {
                    ids.push(id);
                    inserted_rows.push(false);
                    continue;
                }
                let ts = now_ms();
                let meta_s = req.meta.as_ref().map(|m| m.to_string());
                stmt_ins.execute(params![
                    req.uri, req.title, req.text, meta_s, ts, hash_bytes
                ])?;
                let id = tx.last_insert_rowid();
                if let Some(ref emb) = req.embedding {
                    let bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
                    stmt_vec.execute(params![id, bytes])?;
                }
                ids.push(id);
                inserted_rows.push(true);
            }
        }
        tx.commit()?;
        if let Err(e) = crate::sota::ensure_raw_memory_and_enqueue_batch(&mut self.conn, &ids) {
            tracing::warn!("sota raw-memory batch enqueue failed: {e}");
        }
        // PR-A1-wire: mirror new rows into ANN index. Iterate in lockstep:
        // `ids[i]` is either the freshly-inserted rowid for `reqs[i]` OR a
        // de-duplicated existing id (blake3 hash match). Only mirror freshly
        // inserted rows; re-adding existing ids trips usearch's duplicate-key
        // guard and creates noisy, rebuild-looking warnings.
        #[cfg(feature = "ann-usearch")]
        if let Some(ref ann) = self.ann {
            for ((id, req), inserted) in ids.iter().zip(reqs.iter()).zip(inserted_rows.iter()) {
                if !*inserted {
                    continue;
                }
                if let Some(ref emb) = req.embedding
                    && let Err(e) = ann.insert_or_skip(*id, emb)
                {
                    tracing::warn!(
                        "ann batch insert id {id} failed: {e}; sidecar will rebuild on next open"
                    );
                }
            }
        }
        // Turbo: append batch to in-memory ndarray index iff already built.
        #[cfg(feature = "turbo")]
        {
            if let Ok(mut guard) = self.ndarray_search.write()
                && let Some(ref mut idx) = *guard
                && !idx.is_empty()
            {
                for (id, req) in ids.iter().zip(reqs.iter()) {
                    if let Some(ref emb) = req.embedding
                        && let Err(e) = idx.add_row(*id, emb)
                    {
                        tracing::warn!(
                            "turbo ndarray batch add_row id {id} failed: {e}; invalidating cache"
                        );
                        *guard = None;
                        break;
                    }
                }
            }
        }
        // tantivy-fts: mirror batch into persistent index.
        #[cfg(feature = "tantivy-fts")]
        self.mirror_batch_to_tantivy(reqs, &ids);
        Ok(ids)
    }

    /// Tier-1 fast bulk ingest. Skips embedding entirely (vec column = NULL)
    /// for ~663× throughput vs fastembed path. All docs are text-searchable
    /// immediately via FTS5; vec search on these rows returns no results until
    /// a background embed-pass calls `put_batch` with embeddings or directly
    /// inserts into `docs_vec`.
    ///
    /// Rejects any request where `embedding` is `Some` — callers must strip
    /// embeddings before calling this method.
    pub fn put_batch_fast(&mut self, docs: &[PutRequest]) -> Result<Vec<i64>> {
        for req in docs {
            if req.embedding.is_some() {
                return Err(Error::Other(
                    "put_batch_fast: embedding must be None (skip-embed path)".into(),
                ));
            }
        }
        // Temporarily disable fsync — safe because WAL journal ensures recovery
        // on crash. Restored to NORMAL after the transaction commits.
        self.conn.pragma_update(None, "synchronous", "OFF")?;
        let result = (|| -> Result<Vec<i64>> {
            let mut ids = Vec::with_capacity(docs.len());
            let tx = self.conn.transaction()?;
            {
                let mut stmt_chk = tx.prepare("SELECT id FROM docs WHERE blake3 = ?1")?;
                let mut stmt_ins = tx.prepare(
                    "INSERT INTO docs(uri,title,text,meta,ts,blake3) VALUES (?1,?2,?3,?4,?5,?6)",
                )?;
                let ts = now_ms();
                for req in docs {
                    let hash = blake3::hash(req.text.as_bytes());
                    let hash_bytes = hash.as_bytes().to_vec();
                    let found: Option<i64> = stmt_chk
                        .query_row(params![hash_bytes.clone()], |r| r.get(0))
                        .optional()?;
                    if let Some(id) = found {
                        ids.push(id);
                        continue;
                    }
                    let meta_s = req.meta.as_ref().map(|m| m.to_string());
                    stmt_ins.execute(params![
                        req.uri, req.title, req.text, meta_s, ts, hash_bytes
                    ])?;
                    ids.push(tx.last_insert_rowid());
                }
            }
            tx.commit()?;
            Ok(ids)
        })();
        // Always restore synchronous to NORMAL regardless of success/failure.
        let _ = self.conn.pragma_update(None, "synchronous", "NORMAL");
        if let Ok(ref ids) = result
            && let Err(e) = crate::sota::ensure_raw_memory_and_enqueue_batch(&mut self.conn, ids)
        {
            tracing::warn!("sota raw-memory fast-batch enqueue failed: {e}");
        }
        // tantivy-fts: mirror fast-batch into persistent index (deferred commit).
        #[cfg(feature = "tantivy-fts")]
        if let Ok(ref ids) = result {
            self.mirror_batch_to_tantivy(docs, ids);
        }
        result
    }

    /// Tier-2 deferred-FTS bulk ingest. Drops the per-row FTS5 trigger for the
    /// duration of the batch, inserts all rows into `docs`, then rebuilds FTS5
    /// in a single pass. Achieves ~100k+ docs/sec on M4 Max at the cost of FTS5
    /// being unavailable until the merge completes (acceptable for bulk init).
    ///
    /// Rejects any request where `embedding` is `Some`.
    pub fn put_batch_deferred_fts(&mut self, docs: &[PutRequest]) -> Result<Vec<i64>> {
        for req in docs {
            if req.embedding.is_some() {
                return Err(Error::Other(
                    "put_batch_deferred_fts: embedding must be None (skip-embed path)".into(),
                ));
            }
        }
        self.conn.pragma_update(None, "synchronous", "OFF")?;
        let result = (|| -> Result<Vec<i64>> {
            // Drop the AFTER INSERT trigger so FTS5 is not updated per-row.
            self.conn.execute_batch("DROP TRIGGER IF EXISTS docs_ai;")?;

            let mut ids = Vec::with_capacity(docs.len());
            let tx = self.conn.transaction()?;
            let max_before: i64 =
                tx.query_row("SELECT COALESCE(MAX(id),0) FROM docs", [], |r| r.get(0))?;
            {
                let mut stmt_chk = tx.prepare("SELECT id FROM docs WHERE blake3 = ?1")?;
                let mut stmt_ins = tx.prepare(
                    "INSERT INTO docs(uri,title,text,meta,ts,blake3) VALUES (?1,?2,?3,?4,?5,?6)",
                )?;
                let ts = now_ms();
                for req in docs {
                    let hash = blake3::hash(req.text.as_bytes());
                    let hash_bytes = hash.as_bytes().to_vec();
                    let found: Option<i64> = stmt_chk
                        .query_row(params![hash_bytes.clone()], |r| r.get(0))
                        .optional()?;
                    if let Some(id) = found {
                        ids.push(id);
                        continue;
                    }
                    let meta_s = req.meta.as_ref().map(|m| m.to_string());
                    stmt_ins.execute(params![
                        req.uri, req.title, req.text, meta_s, ts, hash_bytes
                    ])?;
                    ids.push(tx.last_insert_rowid());
                }
            }
            // Single-pass FTS5 merge for all newly inserted rows.
            tx.execute(
                "INSERT INTO docs_fts(rowid, title, text) \
                 SELECT id, title, text FROM docs WHERE id > ?1",
                params![max_before],
            )?;
            tx.commit()?;

            // Recreate the AFTER INSERT trigger.
            self.conn.execute_batch(
                "CREATE TRIGGER IF NOT EXISTS docs_ai AFTER INSERT ON docs BEGIN \
                 INSERT INTO docs_fts(rowid, title, text) VALUES (new.id, new.title, new.text); \
                 END;",
            )?;

            Ok(ids)
        })();
        // Always restore synchronous + trigger regardless of outcome.
        let _ = self.conn.pragma_update(None, "synchronous", "NORMAL");
        if result.is_err() {
            let _ = self.conn.execute_batch(
                "CREATE TRIGGER IF NOT EXISTS docs_ai AFTER INSERT ON docs BEGIN \
                 INSERT INTO docs_fts(rowid, title, text) VALUES (new.id, new.title, new.text); \
                 END;",
            );
        }
        if let Ok(ref ids) = result
            && let Err(e) = crate::sota::ensure_raw_memory_and_enqueue_batch(&mut self.conn, ids)
        {
            tracing::warn!("sota raw-memory deferred-batch enqueue failed: {e}");
        }
        // tantivy-fts: mirror deferred batch into persistent index.
        #[cfg(feature = "tantivy-fts")]
        if let Ok(ref ids) = result {
            self.mirror_batch_to_tantivy(docs, ids);
        }
        result
    }

    /// PR-A1-wire: delete a doc by id, removing it from `docs`, `docs_vec`,
    /// `docs_fts`, and (when enabled) the ANN sidecar. Idempotent — returns
    /// `Ok(false)` if the id did not exist.
    pub fn delete(&mut self, id: i64) -> Result<bool> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM memory_edges
             WHERE src_id IN (SELECT id FROM memories WHERE doc_id = ?1)
                OR dst_id IN (SELECT id FROM memories WHERE doc_id = ?1)",
            params![id],
        )?;
        tx.execute(
            "UPDATE memories
             SET superseded_by = NULL
             WHERE superseded_by IN (SELECT id FROM memories WHERE doc_id = ?1)",
            params![id],
        )?;
        tx.execute("DELETE FROM memories WHERE doc_id = ?1", params![id])?;
        tx.execute(
            "DELETE FROM extraction_queue WHERE doc_id = ?1",
            params![id],
        )?;
        let changed: usize = tx.execute("DELETE FROM docs_vec WHERE id = ?1", params![id])?;
        let _ = tx.execute("DELETE FROM docs_fts WHERE rowid = ?1", params![id]);
        let doc_changed = tx.execute("DELETE FROM docs WHERE id = ?1", params![id])?;
        tx.commit()?;
        #[cfg(feature = "ann-usearch")]
        if let Some(ref ann) = self.ann {
            let _ = ann.remove(id);
        }
        // Turbo: invalidate ndarray cache; rebuilt on next search_vec.
        #[cfg(feature = "turbo")]
        if let Ok(mut guard) = self.ndarray_search.write() {
            *guard = None;
        }
        Ok(changed > 0 || doc_changed > 0)
    }

    /// PR-A1-wire: explicit flush of the ANN sidecar to disk. Also called
    /// from `Drop`, but callers may invoke it after heavy write bursts to
    /// bound crash-window exposure.
    #[cfg(feature = "ann-usearch")]
    pub fn flush_ann(&self) -> Result<()> {
        if let Some(ref ann) = self.ann {
            ann.save()?;
        }
        Ok(())
    }

    /// True if ANN index loaded.
    #[cfg(feature = "ann-usearch")]
    pub fn has_ann(&self) -> bool {
        self.ann.is_some()
    }

    /// ANN index entry count (0 if no ann).
    #[cfg(feature = "ann-usearch")]
    pub fn ann_len(&self) -> usize {
        self.ann.as_ref().map(|a| a.len()).unwrap_or(0)
    }

    /// Pre-warm the turbo ndarray search engine.
    /// Call this at startup (before accepting requests) to avoid blocking
    /// the async runtime on the first search request.
    ///
    /// Under `#[cfg(feature = "turbo")]`: loads the full 164K-vector
    /// matrix into memory (~250ms on M4 Max) and pre-normalizes it.
    /// Subsequent `search_vec` calls hit this in-memory index (~5ms)
    /// instead of sqlite-vec brute-force (~112ms).
    #[cfg(feature = "turbo")]
    pub fn warm_turbo(&self) {
        // Fast path: already built
        {
            let guard = self.ndarray_search.read().unwrap();
            if let Some(ref idx) = *guard
                && !idx.is_empty()
            {
                tracing::info!("turbo ndarray_search already warm: {} vectors", idx.len());
                return;
            }
        }
        // Slow path: lazy-build synchronously (caller should do this at startup)
        let mut guard = self.ndarray_search.write().unwrap();
        if guard.is_none() {
            tracing::info!("turbo ndarray_search building from SQL (first-time, ~2s)...");
            match crate::turbo::ndarray_search::NdArraySearch::from_connection(&self.conn) {
                Ok(idx) => {
                    tracing::info!("turbo ndarray_search warmed: {} vectors", idx.len());
                    *guard = Some(idx);
                }
                Err(e) => {
                    tracing::warn!("turbo ndarray_search warm skipped: {e}");
                    *guard = Some(crate::turbo::ndarray_search::NdArraySearch::empty(
                        crate::types::EMBED_DIM,
                    ));
                }
            }
        }
    }

    /// Take the warmed NdArraySearch out of the store's internal RwLock.
    /// Used by the daemon to lift the hot ANN index into a top-level Arc<RwLock<>>
    /// so concurrent reads bypass the Store mutex entirely.
    /// After this call the store's internal cache is empty — put/put_batch will
    /// lazily rebuild it on next write if the turbo feature is active.
    #[cfg(feature = "turbo")]
    pub fn take_ndarray_search(&mut self) -> Option<crate::turbo::ndarray_search::NdArraySearch> {
        self.ndarray_search.write().unwrap().take()
    }

    /// Insert only docs_vec rows with id NOT already in the loaded ANN sidecar.
    /// Used when sidecar exists but is missing tail entries (concurrent puts since last persist).
    /// Avoids "duplicate keys" crash from re-inserting all rows.
    #[cfg(feature = "ann-usearch")]
    fn rebuild_ann_tail(&self, ann: &crate::ann::Ann, deficit: usize) -> Result<()> {
        // Sidecar diffs come from append-only tail writes between daemon exits.
        // Scan only the newest deficit-sized tail and skip duplicate ids defensively.
        let mut stmt = self
            .conn
            .prepare("SELECT id, embedding FROM docs_vec ORDER BY id DESC LIMIT ?1")?;
        let rows = stmt.query_map([deficit as i64], |r| {
            let id: i64 = r.get(0)?;
            let bytes: Vec<u8> = r.get(1)?;
            Ok((id, bytes))
        })?;
        let mut inserted = 0usize;
        for row in rows {
            let (id, bytes) = row?;
            if bytes.len() != EMBED_DIM * 4 {
                continue;
            }
            let mut v = Vec::with_capacity(EMBED_DIM);
            for chunk in bytes.chunks_exact(4) {
                v.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
            }
            if ann.insert_or_skip(id, &v)? {
                inserted += 1;
            }
        }
        tracing::info!("ann rebuild_tail: injected {inserted} new entries");
        Ok(())
    }

    /// PR-A1-wire internal: rebuild the ANN index from `docs_vec` rows.
    /// Called from `Store::open` when the sidecar is missing, corrupt, or
    /// out-of-sync (len < row count).
    #[cfg(feature = "ann-usearch")]
    #[allow(dead_code)]
    fn rebuild_ann_from_docs_vec(&self, ann: &crate::ann::Ann) -> Result<()> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, embedding FROM docs_vec ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            let id: i64 = r.get(0)?;
            let bytes: Vec<u8> = r.get(1)?;
            Ok((id, bytes))
        })?;
        let mut buf: Vec<(i64, Vec<f32>)> = Vec::new();
        for row in rows {
            let (id, bytes) = row?;
            if bytes.len() != EMBED_DIM * 4 {
                return Err(Error::Other(format!(
                    "docs_vec row {id} has {} bytes (expected {})",
                    bytes.len(),
                    EMBED_DIM * 4
                )));
            }
            let mut v = Vec::with_capacity(EMBED_DIM);
            for c in bytes.chunks_exact(4) {
                v.push(f32::from_le_bytes([c[0], c[1], c[2], c[3]]));
            }
            buf.push((id, v));
        }
        ann.rebuild_from_rows(buf)?;
        Ok(())
    }

    pub fn get(&self, id: i64) -> Result<Doc> {
        let doc = self
            .conn
            .query_row(
                "SELECT id,uri,title,text,meta,ts FROM docs WHERE id = ?1",
                params![id],
                map_doc,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("id={}", id)))?;
        Ok(doc)
    }

    #[tracing::instrument(skip(self, query_emb), fields(mode = ?mode, limit))]
    pub fn search(
        &self,
        q: &str,
        mode: SearchMode,
        query_emb: Option<&[f32]>,
        limit: usize,
    ) -> Result<Vec<Hit>> {
        let _t = std::time::Instant::now();
        let op = match mode {
            SearchMode::Lex => "search_lex",
            SearchMode::Vec => "search_vec",
            SearchMode::Hybrid => "search_hybrid",
        };
        let res = match mode {
            SearchMode::Lex => self.search_lex(q, limit),
            SearchMode::Vec => {
                let emb =
                    query_emb.ok_or_else(|| Error::Other("vec search needs embedding".into()))?;
                self.search_vec(emb, limit)
            }
            SearchMode::Hybrid => {
                let emb = query_emb.ok_or_else(|| Error::Other("hybrid needs embedding".into()))?;
                self.search_hybrid(q, emb, limit)
            }
        };
        crate::obs::record_query_duration(op, _t.elapsed().as_secs_f64());
        res
    }

    /// Vector search with metadata filter pushdown.
    ///
    /// **Strategy — Option 1 (ef-boost oversampling):**
    /// usearch has no per-candidate callback API, so we cannot intercept
    /// HNSW traversal. Instead we boost `ef_search` proportional to filter
    /// selectivity (inverse fraction expected to pass), oversample, then
    /// post-filter. This gives correct recall without reimplementing HNSW.
    ///
    /// ef_mult = ceil(1 / selectivity), clamped [2, 32].
    /// oversample_k = limit * ef_mult.
    ///
    /// When `opts.filter` is None this is identical to `search_vec`.
    pub fn search_vec_filtered(
        &self,
        emb: &[f32],
        limit: usize,
        opts: &SearchOptions,
    ) -> Result<Vec<Hit>> {
        let pred = match &opts.filter {
            None => return self.search_vec(emb, limit),
            Some(p) => p,
        };
        let selectivity = pred.estimated_selectivity().max(0.01);
        let ef_mult = opts
            .ef_multiplier
            .unwrap_or_else(|| ((1.0 / selectivity).ceil() as usize).clamp(2, 32));
        let oversample_k = (limit * ef_mult).max(limit + 1);

        // ANN oversample with ef boost.
        let candidates = self.search_vec_oversampled(emb, oversample_k, ef_mult)?;

        // Fetch meta for all candidates in one SQL round-trip.
        self.filter_hits_by_meta(candidates, pred, limit)
    }

    /// Like `search_filtered` but for hybrid mode (lex+vec with filter).
    pub fn search_hybrid_filtered(
        &self,
        q: &str,
        emb: &[f32],
        limit: usize,
        opts: &SearchOptions,
    ) -> Result<Vec<Hit>> {
        let pred = match &opts.filter {
            None => return self.search_hybrid(q, emb, limit),
            Some(p) => p,
        };
        let selectivity = pred.estimated_selectivity().max(0.01);
        let ef_mult = opts
            .ef_multiplier
            .unwrap_or_else(|| ((1.0 / selectivity).ceil() as usize).clamp(2, 32));
        let oversample_k = (limit * ef_mult).max(limit + 1);
        let k = oversample_k;
        let lex = self.search_lex(q, k).unwrap_or_default();
        let vec = self.search_vec_oversampled(emb, k, ef_mult)?;
        // RRF merge via NEON (oversample then filter)
        let merged = rrf_merge_neon(lex, vec, oversample_k);
        // now filter
        let ids: Vec<i64> = merged.iter().map(|h| h.id).collect();
        let meta_map = self.fetch_meta_by_ids(&ids)?;
        let out: Vec<Hit> = merged
            .into_iter()
            .filter(|h| {
                let meta_val = meta_map
                    .get(&h.id)
                    .and_then(|s| s.as_ref())
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
                pred.matches(meta_val.as_ref())
            })
            .take(limit)
            .collect();
        Ok(out)
    }

    /// Public unified search with filter.
    pub fn search_with_options(
        &self,
        q: &str,
        mode: SearchMode,
        query_emb: Option<&[f32]>,
        limit: usize,
        opts: &SearchOptions,
    ) -> Result<Vec<Hit>> {
        if opts.filter.is_none() {
            return self.search(q, mode, query_emb, limit);
        }
        match mode {
            SearchMode::Lex => {
                // For lex: post-filter after oversample.
                let pred = opts.filter.as_ref().unwrap();
                let selectivity = pred.estimated_selectivity().max(0.01);
                let ef_mult = opts
                    .ef_multiplier
                    .unwrap_or_else(|| ((1.0 / selectivity).ceil() as usize).clamp(2, 32));
                let candidates = self.search_lex(q, limit * ef_mult)?;
                self.filter_hits_by_meta(candidates, pred, limit)
            }
            SearchMode::Vec => {
                let emb =
                    query_emb.ok_or_else(|| Error::Other("vec search needs embedding".into()))?;
                self.search_vec_filtered(emb, limit, opts)
            }
            SearchMode::Hybrid => {
                let emb = query_emb.ok_or_else(|| Error::Other("hybrid needs embedding".into()))?;
                self.search_hybrid_filtered(q, emb, limit, opts)
            }
        }
    }

    /// Oversample via ANN/vec with boosted ef, return `Hit`s.
    fn search_vec_oversampled(
        &self,
        emb: &[f32],
        k: usize,
        #[cfg_attr(not(feature = "ann-usearch"), allow(unused_variables))] ef_mult: usize,
    ) -> Result<Vec<Hit>> {
        if emb.len() != EMBED_DIM {
            return Err(Error::DimMismatch {
                expected: EMBED_DIM,
                got: emb.len(),
            });
        }

        #[cfg(feature = "ann-usearch")]
        if let Some(ref ann) = self.ann
            && !ann.is_empty()
        {
            // Boost ef proportional to multiplier.
            let boosted_ef = (ann.expansion_search() * ef_mult).min(4096).max(k);
            match ann.search_with_ef(emb, k, boosted_ef) {
                Ok(hits) if !hits.is_empty() => {
                    return self.hydrate_hits_from_ann(&hits);
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("ann oversampled search fell back: {e}");
                }
            }
        }

        // Fallback: sqlite-vec with larger k.
        let bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
        let sql = "SELECT d.id,d.uri,d.title,d.text,v.distance,d.meta,d.ts
                   FROM docs_vec v JOIN docs d ON d.id = v.id
                   WHERE v.embedding MATCH ?1 AND k = ?2
                   ORDER BY v.distance";
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params![bytes, k as i64], |r| {
            Ok(Hit {
                id: r.get(0)?,
                uri: r.get(1)?,
                title: r.get(2)?,
                text: r.get(3)?,
                score: 1.0 / (1.0 + r.get::<_, f64>(4)?),
                meta: parse_meta_cell(r.get::<_, Option<String>>(5)?),
                ts: Some(r.get(6)?),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Fetch `meta` column (raw JSON string) for a list of doc ids.
    /// Returns a map id → Option<String>.
    fn fetch_meta_by_ids(
        &self,
        ids: &[i64],
    ) -> Result<std::collections::HashMap<i64, Option<String>>> {
        if ids.is_empty() {
            return Ok(Default::default());
        }
        let placeholders = (0..ids.len())
            .map(|i| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!("SELECT id, meta FROM docs WHERE id IN ({placeholders})");
        let mut stmt = self.conn.prepare(&sql)?;
        let params_iter: Vec<&dyn rusqlite::ToSql> =
            ids.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
        let rows = stmt.query_map(params_iter.as_slice(), |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
        })?;
        let mut map = std::collections::HashMap::new();
        for row in rows {
            let (id, meta) = row?;
            map.insert(id, meta);
        }
        Ok(map)
    }

    /// Given a set of candidate Hits, fetch their meta and keep only those
    /// matching `pred`, up to `limit`.
    fn filter_hits_by_meta(
        &self,
        candidates: Vec<Hit>,
        pred: &MetadataPredicate,
        limit: usize,
    ) -> Result<Vec<Hit>> {
        let ids: Vec<i64> = candidates.iter().map(|h| h.id).collect();
        let meta_map = self.fetch_meta_by_ids(&ids)?;
        let out = candidates
            .into_iter()
            .filter(|h| {
                let meta_val = meta_map
                    .get(&h.id)
                    .and_then(|s| s.as_ref())
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
                pred.matches(meta_val.as_ref())
            })
            .take(limit)
            .collect();
        Ok(out)
    }

    #[tracing::instrument(skip(self), fields(limit))]
    fn search_lex(&self, q: &str, limit: usize) -> Result<Vec<Hit>> {
        let Some(fts_query) = fts5_match_query(q) else {
            return Ok(Vec::new());
        };
        #[cfg(feature = "tantivy-fts")]
        {
            let mut guard = self.tantivy_fts.lock();
            if guard.is_none() {
                // Fallback lazy build (e.g. encrypted store or test path).
                // Open persistent index if it exists, else in-RAM.
                let fts_path = &self.tantivy_path;
                let ram_path = std::path::Path::new(":memory:");
                let use_path = if fts_path.exists() {
                    fts_path.as_path()
                } else {
                    ram_path
                };
                let mut fts = synapse_fts::FtsIndex::new(use_path)
                    .map_err(|e| Error::Other(e.to_string()))?;
                let last_id = fts.last_indexed_doc_id();
                let mut stmt = self
                    .conn
                    .prepare("SELECT id, title, text FROM docs WHERE id > ?1 ORDER BY id ASC")?;
                let rows: Vec<(i64, Option<String>, String)> = stmt
                    .query_map(params![last_id as i64], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                    })?
                    .collect::<rusqlite::Result<_>>()?;
                let new_max = rows.last().map(|(id, _, _)| *id as u64).unwrap_or(last_id);
                for (id, title, text) in rows {
                    let combined = format!("{} {}", title.as_deref().unwrap_or(""), text);
                    fts.add(id as u64, &combined)
                        .map_err(|e| Error::Other(e.to_string()))?;
                }
                fts.commit().map_err(|e| Error::Other(e.to_string()))?;
                if new_max > last_id {
                    let _ = fts.set_last_indexed_doc_id(new_max);
                }
                *guard = Some(fts);
            }
            let fts = guard.as_mut().unwrap();
            // Commit any pending (deferred) adds before searching.
            if let Err(e) = fts.commit() {
                tracing::warn!("tantivy deferred commit: {e}");
            }
            // Persist the max indexed doc_id after commit.
            {
                let max_id: i64 = self
                    .conn
                    .query_row("SELECT COALESCE(MAX(id),0) FROM docs", [], |r| r.get(0))
                    .unwrap_or(0);
                if max_id > 0 {
                    let _ = fts.set_last_indexed_doc_id(max_id as u64);
                }
            }
            let results = fts
                .search(&fts_query, limit)
                .map_err(|e| Error::Other(e.to_string()))?;
            if results.is_empty() {
                return Ok(vec![]);
            }
            // Hydrate Hits from SQLite by id.
            let mut hits = Vec::with_capacity(results.len());
            for (doc_id, score) in results {
                let row = self.conn.query_row(
                    "SELECT id,uri,title,text,meta,ts FROM docs WHERE id = ?1",
                    params![doc_id as i64],
                    |r| {
                        Ok(Hit {
                            id: r.get(0)?,
                            uri: r.get(1)?,
                            title: r.get(2)?,
                            text: r.get(3)?,
                            score: score as f64,
                            meta: parse_meta_cell(r.get::<_, Option<String>>(4)?),
                            ts: Some(r.get(5)?),
                        })
                    },
                );
                match row {
                    Ok(h) => hits.push(h),
                    Err(rusqlite::Error::QueryReturnedNoRows) => {}
                    Err(e) => return Err(e.into()),
                }
            }
            Ok(hits)
        }
        #[cfg(not(feature = "tantivy-fts"))]
        {
            let sql = "SELECT d.id,d.uri,d.title,d.text,bm25(docs_fts) as score,d.meta,d.ts
                       FROM docs_fts JOIN docs d ON d.id = docs_fts.rowid
                       WHERE docs_fts MATCH ?1
                       ORDER BY score LIMIT ?2";
            let mut stmt = self.conn.prepare(sql)?;
            let rows = stmt.query_map(params![fts_query, limit as i64], |r| {
                Ok(Hit {
                    id: r.get(0)?,
                    uri: r.get(1)?,
                    title: r.get(2)?,
                    text: r.get(3)?,
                    score: r.get::<_, f64>(4).map(|s| -s)?,
                    meta: parse_meta_cell(r.get::<_, Option<String>>(5)?),
                    ts: Some(r.get(6)?),
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        }
    }

    #[tracing::instrument(skip(self, emb), fields(limit))]
    fn search_vec(&self, emb: &[f32], limit: usize) -> Result<Vec<Hit>> {
        let _t = std::time::Instant::now();
        if emb.len() != EMBED_DIM {
            return Err(Error::DimMismatch {
                expected: EMBED_DIM,
                got: emb.len(),
            });
        }

        // PR-A1-wire: usearch ANN fast-path. On any ANN error we fall back
        // to the brute-force sqlite-vec path below, so correctness is
        // preserved even if the sidecar is stale/broken.
        #[cfg(feature = "ann-usearch")]
        if let Some(ref ann) = self.ann
            && !ann.is_empty()
        {
            match ann.search(emb, limit) {
                Ok(hits) if !hits.is_empty() => {
                    return self.hydrate_hits_from_ann(&hits);
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("ann search fell back to sqlite-vec: {e}");
                }
            }
        }

        // Turbo fast-path: in-memory ndarray brute-force kNN.
        // Build cache lazily on first call (one-time ~250ms scan @ 162k×384).
        // Subsequent calls hit the matrix directly (~7ms p50 @ 162k vs ~50ms sqlite-vec).
        #[cfg(feature = "turbo")]
        {
            // Fast path: cache already built.
            {
                let guard = self.ndarray_search.read().unwrap();
                if let Some(ref idx) = *guard
                    && !idx.is_empty()
                {
                    // Binary cascade: Hamming pre-filter 164k→4096, then f32 rerank.
                    // Falls back to full search when corpus < 4096 (binary_k clamped).
                    // binary_k=4096: R@10=0.994, 3.3× faster than full scan.
                    // Raise to 8192 if corpus <8k to avoid edge cases.
                    let binary_k = 4096usize.max(limit * 64).min(idx.len());
                    let pairs = if binary_k < idx.len() {
                        idx.search_cascade(emb, limit, binary_k)
                    } else {
                        idx.search(emb, limit)
                    };
                    if !pairs.is_empty() {
                        crate::obs::inc_cache_hit();
                        crate::obs::record_visited_nodes(pairs.len() as u64);
                        crate::obs::record_query_duration("search_vec", _t.elapsed().as_secs_f64());
                        return self.hydrate_hits_by_id_dist(&pairs);
                    }
                }
            }
            // Slow path: lazy-build, then retry.
            {
                let mut guard = self.ndarray_search.write().unwrap();
                if guard.is_none() {
                    crate::obs::inc_cache_miss();
                    match crate::turbo::ndarray_search::NdArraySearch::from_connection(&self.conn) {
                        Ok(idx) => {
                            tracing::info!("turbo ndarray_search built: {} vectors", idx.len());
                            *guard = Some(idx);
                        }
                        Err(e) => {
                            // Empty DB or other failure — install empty index so we
                            // do not retry on every call. Falls through to sqlite-vec.
                            tracing::debug!("turbo ndarray_search build skipped: {e}");
                            *guard = Some(crate::turbo::ndarray_search::NdArraySearch::empty(
                                EMBED_DIM,
                            ));
                        }
                    }
                }
                if let Some(ref idx) = *guard
                    && !idx.is_empty()
                {
                    // binary_k=4096: R@10=0.994, 3.3× faster than full scan.
                    // Raise to 8192 if corpus <8k to avoid edge cases.
                    let binary_k = 4096usize.max(limit * 64).min(idx.len());
                    let pairs = if binary_k < idx.len() {
                        idx.search_cascade(emb, limit, binary_k)
                    } else {
                        idx.search(emb, limit)
                    };
                    drop(guard);
                    if !pairs.is_empty() {
                        return self.hydrate_hits_by_id_dist(&pairs);
                    }
                }
            }
        }

        let bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
        let sql = "SELECT d.id,d.uri,d.title,d.text,v.distance,d.meta,d.ts
                   FROM docs_vec v JOIN docs d ON d.id = v.id
                   WHERE v.embedding MATCH ?1 AND k = ?2
                   ORDER BY v.distance";
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params![bytes, limit as i64], |r| {
            Ok(Hit {
                id: r.get(0)?,
                uri: r.get(1)?,
                title: r.get(2)?,
                text: r.get(3)?,
                score: 1.0 / (1.0 + r.get::<_, f64>(4)?),
                meta: parse_meta_cell(r.get::<_, Option<String>>(5)?),
                ts: Some(r.get(6)?),
            })
        })?;
        let hits = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        crate::obs::record_query_duration("search_vec", _t.elapsed().as_secs_f64());
        Ok(hits)
    }

    /// Vector search with explicit backend selection (auto-routed by `target_recall`).
    ///
    /// | Backend       | binary_k | QPS   | R@10  |
    /// |---------------|----------|-------|-------|
    /// | Cascade       | 4096     | 697   | 0.994 |
    /// | UsearchHnsw   | —        | 1631  | 0.982 |
    /// | BinaryFirst   | small    | 4845  | 0.888 |
    ///
    /// Falls back to `search_vec` (Cascade) when turbo is disabled or the
    /// index is not loaded yet.
    pub fn search_vec_with_backend(
        &self,
        emb: &[f32],
        limit: usize,
        #[cfg_attr(not(feature = "turbo"), allow(unused_variables))] backend: SearchBackend,
    ) -> Result<Vec<Hit>> {
        #[cfg(feature = "turbo")]
        {
            let guard = self.ndarray_search.read().unwrap();
            if let Some(ref idx) = *guard
                && !idx.is_empty()
            {
                let pairs = match backend {
                    SearchBackend::Cascade => {
                        let binary_k = 4096usize.max(limit * 64).min(idx.len());
                        if binary_k < idx.len() {
                            idx.search_cascade(emb, limit, binary_k)
                        } else {
                            idx.search(emb, limit)
                        }
                    }
                    SearchBackend::BinaryFirst => {
                        // Small binary_k → high throughput, lower recall.
                        let binary_k = (limit * 16).max(64).min(idx.len());
                        if binary_k < idx.len() {
                            idx.search_cascade(emb, limit, binary_k)
                        } else {
                            idx.search(emb, limit)
                        }
                    }
                    SearchBackend::UsearchHnsw => {
                        // Attempt usearch if feature enabled; otherwise cascade at M=48 ef=64 equivalent.
                        #[cfg(feature = "ann-usearch")]
                        if let Some(ref ann) = self.ann
                            && !ann.is_empty()
                        {
                            match ann.search(emb, limit) {
                                Ok(hits) if !hits.is_empty() => {
                                    drop(guard);
                                    return self.hydrate_hits_from_ann(&hits);
                                }
                                _ => {}
                            }
                        }
                        // Fallback: cascade with moderate binary_k (R≈0.98).
                        let binary_k = 2048usize.max(limit * 32).min(idx.len());
                        if binary_k < idx.len() {
                            idx.search_cascade(emb, limit, binary_k)
                        } else {
                            idx.search(emb, limit)
                        }
                    }
                };
                if !pairs.is_empty() {
                    return self.hydrate_hits_by_id_dist(&pairs);
                }
            }
        }
        self.search_vec(emb, limit)
    }

    /// Exact-rerank guarantee: full brute-force cosine scan (R@N = 1.0).
    ///
    /// Skips the Hamming pre-filter cascade and scans every vector in the
    /// in-memory matrix. +0.5–2 ms vs cascade on a 10k corpus.
    /// Falls back to `search_vec` when the turbo index is not available.
    pub fn search_vec_exact(&self, emb: &[f32], limit: usize) -> Result<Vec<Hit>> {
        if emb.len() != EMBED_DIM {
            return Err(Error::DimMismatch {
                expected: EMBED_DIM,
                got: emb.len(),
            });
        }
        #[cfg(feature = "turbo")]
        {
            let guard = self.ndarray_search.read().unwrap();
            if let Some(ref idx) = *guard
                && !idx.is_empty()
            {
                // idx.search() is full brute-force — no Hamming pre-filter.
                let pairs = idx.search(emb, limit);
                if !pairs.is_empty() {
                    return self.hydrate_hits_by_id_dist(&pairs);
                }
            }
        }
        self.search_vec(emb, limit)
    }

    /// Auto-build the knowledge graph on ingest: relate a freshly-stored doc to its
    /// top-k nearest neighbours with bidirectional "similar" edges, so `ground`,
    /// `graph traverse`, and hippo retrieval work WITHOUT manual `graph relate`.
    /// Weak links (cosine below `min_sim`) are skipped to keep the graph signal-rich.
    /// Best-effort + idempotent (INSERT OR REPLACE); requires feature `hippo`.
    /// Read-only: top-k nearest neighbours as (id, rank-weight), excluding `new_id`.
    /// Split from the edge writes so the daemon can RELEASE the global store lock
    /// between the expensive vector search and the writes — avoids holding the lock
    /// across a full-corpus scan on every put (reader-starvation guard).
    #[cfg(feature = "hippo")]
    pub fn similar_neighbors(&self, new_id: i64, emb: &[f32], k: usize) -> Vec<(i64, f64)> {
        let hits = match self.search_vec_exact(emb, k + 1) {
            Ok(h) => h,
            Err(_) => return Vec::new(),
        };
        let mut out = Vec::with_capacity(k);
        for (rank, h) in hits.into_iter().enumerate() {
            if h.id == new_id {
                continue;
            }
            out.push((h.id, 1.0 / (1.0 + rank as f64))); // weight decays with proximity rank
            if out.len() >= k {
                break;
            }
        }
        out
    }

    /// Write-only: bidirectional "similar" edges for precomputed neighbours.
    /// Fast (only INSERTs) so the store lock is held only briefly.
    #[cfg(feature = "hippo")]
    pub fn relate_similar(&self, new_id: i64, neighbors: &[(i64, f64)]) -> usize {
        let _ = synapse_graph::ensure_schema(&self.conn);
        let mut n = 0usize;
        for &(to, w) in neighbors {
            let _ = synapse_graph::relate(&self.conn, new_id, to, "similar", w, None);
            let _ = synapse_graph::relate(&self.conn, to, new_id, "similar", w, None);
            n += 1;
        }
        n
    }

    /// Convenience: search + relate in one call (holds the lock for both — prefer the
    /// split `similar_neighbors` + `relate_similar` on the hot path).
    #[cfg(feature = "hippo")]
    pub fn auto_relate(&self, new_id: i64, emb: &[f32], k: usize, _min_sim: f64) -> Result<usize> {
        let nb = self.similar_neighbors(new_id, emb, k);
        Ok(self.relate_similar(new_id, &nb))
    }

    /// PR-A1-wire helper: given `(id, distance)` from the ANN, fetch full
    /// `Hit` records (uri/title/text) from SQL. One round-trip, preserved order.
    #[cfg(feature = "ann-usearch")]
    fn hydrate_hits_from_ann(&self, ann_hits: &[(i64, f32)]) -> Result<Vec<Hit>> {
        if ann_hits.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (0..ann_hits.len())
            .map(|i| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(",");
        let sql =
            format!("SELECT id,uri,title,text,meta,ts FROM docs WHERE id IN ({placeholders})");
        let mut stmt = self.conn.prepare(&sql)?;
        let ids: Vec<i64> = ann_hits.iter().map(|(i, _)| *i).collect();
        let params_iter: Vec<&dyn rusqlite::ToSql> =
            ids.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
        let mut by_id: std::collections::HashMap<
            i64,
            (
                Option<String>,
                Option<String>,
                String,
                Option<serde_json::Value>,
                i64,
            ),
        > = Default::default();
        let rows = stmt.query_map(params_iter.as_slice(), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                parse_meta_cell(r.get::<_, Option<String>>(4)?),
                r.get::<_, i64>(5)?,
            ))
        })?;
        for row in rows {
            let (id, uri, title, text, meta, ts) = row?;
            by_id.insert(id, (uri, title, text, meta, ts));
        }
        let dists: Vec<f32> = ann_hits.iter().map(|(_, d)| *d).collect();
        #[cfg(feature = "turbo")]
        let scores: Vec<f32> = distance_to_score(&dists);
        #[cfg(not(feature = "turbo"))]
        let scores: Vec<f32> = dists.iter().map(|d| 1.0_f32 / (1.0_f32 + d)).collect();
        let mut out = Vec::with_capacity(ann_hits.len());
        for ((id, _), score) in ann_hits.iter().zip(scores.iter()) {
            if let Some((uri, title, text, meta, ts)) = by_id.remove(id) {
                out.push(Hit {
                    id: *id,
                    uri,
                    title,
                    text,
                    score: *score as f64,
                    meta,
                    ts: Some(ts),
                });
            }
        }
        Ok(out)
    }

    /// Turbo helper: given `(id, distance)` pairs from the ndarray index,
    /// fetch full `Hit` records (uri/title/text) from SQL in one round-trip.
    /// Preserves input order. Used by the turbo fast-path in `search_vec`.
    #[cfg(feature = "turbo")]
    pub fn hydrate_hits_by_id_dist(&self, pairs: &[(i64, f32)]) -> Result<Vec<Hit>> {
        if pairs.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (0..pairs.len())
            .map(|i| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(",");
        let sql =
            format!("SELECT id,uri,title,text,meta,ts FROM docs WHERE id IN ({placeholders})");
        let mut stmt = self.conn.prepare(&sql)?;
        let ids: Vec<i64> = pairs.iter().map(|(i, _)| *i).collect();
        let params_iter: Vec<&dyn rusqlite::ToSql> =
            ids.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
        let mut by_id: std::collections::HashMap<
            i64,
            (
                Option<String>,
                Option<String>,
                String,
                Option<serde_json::Value>,
                i64,
            ),
        > = Default::default();
        let rows = stmt.query_map(params_iter.as_slice(), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                parse_meta_cell(r.get::<_, Option<String>>(4)?),
                r.get::<_, i64>(5)?,
            ))
        })?;
        for row in rows {
            let (id, uri, title, text, meta, ts) = row?;
            by_id.insert(id, (uri, title, text, meta, ts));
        }
        let mut out = Vec::with_capacity(pairs.len());
        for (id, dist) in pairs.iter() {
            if let Some((uri, title, text, meta, ts)) = by_id.remove(id) {
                out.push(Hit {
                    id: *id,
                    uri,
                    title,
                    text,
                    score: 1.0_f64 / (1.0_f64 + *dist as f64),
                    meta,
                    ts: Some(ts),
                });
            }
        }
        Ok(out)
    }

    #[tracing::instrument(skip(self, emb), fields(limit))]
    fn search_hybrid(&self, q: &str, emb: &[f32], limit: usize) -> Result<Vec<Hit>> {
        let k = limit * 3;
        let lex = self.search_lex(q, k).unwrap_or_default();
        let vec = self.search_vec(emb, k).unwrap_or_default();
        Ok(rrf_merge_neon(lex, vec, limit))
    }

    /// HippoRAG-2 hybrid search: vec/lex hybrid + graph PPR, RRF-merged.
    ///
    /// `alpha_graph` in [0,1]: 0 = pure vec-hybrid, 1 = pure graph signal.
    /// Typical sweet-spot: 0.3–0.5 (+20-30% recall on entity-centric queries).
    ///
    /// Requires feature `hippo` on the synapse-graph crate AND the SOTA schema
    /// (entities / memories / memory_edges) to be migrated on this connection.
    #[cfg(feature = "hippo")]
    pub fn search_hybrid_hippo(
        &self,
        q: &str,
        emb: &[f32],
        limit: usize,
        alpha_graph: f32,
    ) -> Result<Vec<Hit>> {
        use synapse_graph::hippo::{hippo_retrieve, rrf_hippo};

        let k = limit * 3;

        // 1. Standard vec-hybrid.
        let lex = self.search_lex(q, k).unwrap_or_default();
        let vec = self.search_vec(emb, k).unwrap_or_default();
        let hybrid: Vec<(i64, f64)> = rrf_merge_neon(lex, vec, k)
            .into_iter()
            .map(|h| (h.id, h.score))
            .collect();

        // 2. Graph PPR over KG.
        let hippo = hippo_retrieve(&self.conn, q, k, alpha_graph, 10).unwrap_or_default();

        // 3. RRF merge.
        let merged = rrf_hippo(&hybrid, &hippo, alpha_graph, limit);

        // 4. Fetch Hit payloads for merged doc_ids.
        let ids: Vec<i64> = merged.iter().map(|(id, _)| *id).collect();
        let score_map: std::collections::HashMap<i64, f64> = merged.into_iter().collect();

        let mut out: Vec<Hit> = Vec::with_capacity(ids.len());
        for id in &ids {
            if let Ok(row) = self.conn.query_row(
                "SELECT id, uri, title, text, meta, ts FROM docs WHERE id = ?1",
                rusqlite::params![id],
                |r| {
                    Ok(Hit {
                        id: r.get(0)?,
                        uri: r.get(1)?,
                        title: r.get(2)?,
                        text: r.get(3)?,
                        score: 0.0,
                        meta: parse_meta_cell(r.get::<_, Option<String>>(4)?),
                        ts: Some(r.get(5)?),
                    })
                },
            ) {
                let score = score_map.get(&row.id).copied().unwrap_or(0.0);
                out.push(Hit { score, ..row });
            }
        }
        out.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(out)
    }

    /// Open or create an encrypted (SQLCipher) database using a raw 32-byte key
    /// derived by the caller via `derive_brain_key`. The key is passed directly
    /// as `PRAGMA key="x'<hex>'"` before any other SQL; no KDF is applied here.
    ///
    /// Requires feature `encryption`.
    #[cfg(feature = "encryption")]
    pub fn open_with_brain_key(path: impl AsRef<Path>, key: &[u8; 32]) -> Result<Self> {
        let key_hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
                *const (),
                SqliteAutoExtensionFn,
            >(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
        let conn = Connection::open(path.as_ref())?;
        conn.pragma_update(None, "key", format!("x'{key_hex}'"))?;
        // Verify the key is correct by attempting a read; SQLCipher will return
        // SQLITE_NOTADB / error 26 if the key is wrong.
        conn.execute_batch("SELECT count(*) FROM sqlite_master;")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "busy_timeout", 10000_i64)?;
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        conn.pragma_update(None, "mmap_size", 268_435_456_i64)?;
        conn.pragma_update(None, "cache_size", -65536_i64)?;
        conn.pragma_update(None, "wal_autocheckpoint", 0_i64)?;
        crate::sql_fns::register_synapse_match(&conn)?;
        let s = Self::from_conn(conn);
        s.migrate()?;
        Ok(s)
    }

    /// Verify the Ed25519 signature on a doc. Returns Err if no sig or invalid.
    pub fn verify(&self, id: i64, vk: &ed25519_dalek::VerifyingKey) -> Result<()> {
        let (text, sig_opt): (String, Option<Vec<u8>>) = self
            .conn
            .query_row(
                "SELECT text, sig FROM docs WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("id={}", id)))?;
        let sig_bytes = sig_opt.ok_or_else(|| Error::Other("doc has no signature".into()))?;
        let arr: [u8; 64] = sig_bytes
            .try_into()
            .map_err(|_| Error::Other("bad sig length".into()))?;
        let hash = blake3::hash(text.as_bytes());
        crate::sign::verify_bytes(vk, hash.as_bytes(), &arr)
    }

    /// Return docs ordered by timestamp descending (for timeline view).
    pub fn timeline(&self, limit: usize, offset: usize) -> Result<Vec<crate::types::Doc>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, uri, title, text, meta, ts FROM docs ORDER BY ts DESC LIMIT ?1 OFFSET ?2",
        )?;
        let docs = stmt
            .query_map(params![limit as i64, offset as i64], map_doc)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(docs)
    }

    /// Return docs whose event time falls in a Unix-second range.
    ///
    /// `meta.occurred_ts` is authoritative when present; insertion time is the
    /// compatibility fallback for older documents. `docs.ts` is stored in ms.
    pub fn timeline_between(
        &self,
        lo_secs: i64,
        hi_secs: i64,
        limit: usize,
    ) -> Result<Vec<crate::types::Doc>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, uri, title, text, meta, ts
             FROM docs
             WHERE CAST(COALESCE(
                       CASE WHEN json_valid(meta) THEN json_extract(meta, '$.occurred_ts') END,
                       ts / 1000
                   ) AS INTEGER) BETWEEN ?1 AND ?2
             ORDER BY CAST(COALESCE(
                       CASE WHEN json_valid(meta) THEN json_extract(meta, '$.occurred_ts') END,
                       ts / 1000
                   ) AS INTEGER) DESC
             LIMIT ?3",
        )?;
        let docs = stmt
            .query_map(params![lo_secs, hi_secs, limit as i64], map_doc)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(docs)
    }

    pub fn stats(&self) -> Result<Stats> {
        let docs: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM docs", [], |r| r.get(0))?;
        let vecs: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM docs_vec", [], |r| r.get(0))?;
        Ok(Stats { docs, vecs })
    }
}

/// PR-A1-wire: best-effort sidecar flush on drop. Any error is logged but
/// cannot be returned — Drop has no result. Callers who require a confirmed
/// flush should call `flush_ann()` explicitly.
#[cfg(feature = "ann-usearch")]
impl Drop for Store {
    fn drop(&mut self) {
        if let Some(ref ann) = self.ann
            && let Err(e) = ann.save()
        {
            tracing::warn!(
                "ann drop-save failed: {e}; sidecar may be stale, but docs_vec is authoritative"
            );
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Stats {
    pub docs: i64,
    pub vecs: i64,
}

fn map_doc(r: &rusqlite::Row) -> rusqlite::Result<Doc> {
    let meta: Option<String> = r.get(4)?;
    Ok(Doc {
        id: r.get(0)?,
        uri: r.get(1)?,
        title: r.get(2)?,
        text: r.get(3)?,
        meta: meta.and_then(|s| serde_json::from_str(&s).ok()),
        ts: r.get(5)?,
    })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_emb(seed: u8) -> Vec<f32> {
        (0..EMBED_DIM)
            .map(|i| ((i as u8).wrapping_mul(seed) as f32) / 255.0)
            .collect()
    }

    #[test]
    fn open_migrate_put_lex() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut s = Store::open(tmp.path()).unwrap();
        let id = s
            .put(&PutRequest {
                title: Some("t".into()),
                text: "rust sqlite fts5 vector memory".into(),
                embedding: Some(fake_emb(7)),
                ..Default::default()
            })
            .unwrap();
        assert!(id > 0);
        let hits = s.search("sqlite", SearchMode::Lex, None, 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, id);
    }

    #[test]
    fn timeline_between_prefers_event_time_over_capture_time() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut store = Store::open(tmp.path()).unwrap();
        let event_ts = 1_735_689_600_i64; // 2025-01-01 UTC
        let id = store
            .put(&PutRequest {
                text: "historical release decision".into(),
                meta: Some(serde_json::json!({"occurred_ts": event_ts})),
                ..Default::default()
            })
            .unwrap();
        let hits = store
            .timeline_between(event_ts, event_ts + 86_399, 10)
            .unwrap();
        assert_eq!(hits.iter().map(|doc| doc.id).collect::<Vec<_>>(), vec![id]);
        assert!(
            store
                .timeline_between(event_ts + 86_400, event_ts + 172_799, 10)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn lex_search_sanitizes_agent_strings() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut s = Store::open(tmp.path()).unwrap();
        let id = s
            .put(&PutRequest {
                title: Some("verify".into()),
                text: "sota auto memory verification path".into(),
                embedding: Some(fake_emb(7)),
                ..Default::default()
            })
            .unwrap();
        let hits = s
            .search("sota-auto-memory", SearchMode::Lex, None, 10)
            .unwrap();
        assert!(hits.iter().any(|h| h.id == id));
    }

    #[test]
    fn dedup_same_text() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut s = Store::open(tmp.path()).unwrap();
        let r = PutRequest {
            text: "same text".into(),
            ..Default::default()
        };
        let a = s.put(&r).unwrap();
        let b = s.put(&r).unwrap();
        assert_eq!(a, b);
        assert_eq!(s.stats().unwrap().docs, 1);
    }

    #[test]
    fn put_auto_wires_sota_memory_once() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut s = Store::open(tmp.path()).unwrap();
        let req = PutRequest {
            text: "auto memory bridge document".into(),
            ..Default::default()
        };
        let id = s.put(&req).unwrap();
        assert_eq!(s.put(&req).unwrap(), id);

        let raw_count: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM memories
                 WHERE doc_id = ?1 AND memory_type = 'raw' AND entity_id IS NULL",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        let queue_count: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM extraction_queue WHERE doc_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();

        assert_eq!(raw_count, 1);
        assert_eq!(queue_count, 1);
    }

    #[test]
    fn put_batch_fast_auto_wires_sota_memory() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut s = Store::open(tmp.path()).unwrap();
        let reqs: Vec<PutRequest> = (0..3)
            .map(|i| PutRequest {
                text: format!("fast auto memory bridge document {i}"),
                ..Default::default()
            })
            .collect();
        let ids = s.put_batch_fast(&reqs).unwrap();

        let raw_count: i64 = s
            .conn
            .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
            .unwrap();
        let queue_count: i64 = s
            .conn
            .query_row("SELECT COUNT(*) FROM extraction_queue", [], |r| r.get(0))
            .unwrap();

        assert_eq!(ids.len(), 3);
        assert_eq!(raw_count, 3);
        assert_eq!(queue_count, 3);
    }

    #[test]
    fn vec_search() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut s = Store::open(tmp.path()).unwrap();
        let e1 = fake_emb(1);
        let e2 = fake_emb(2);
        s.put(&PutRequest {
            text: "a".into(),
            embedding: Some(e1.clone()),
            ..Default::default()
        })
        .unwrap();
        s.put(&PutRequest {
            text: "b".into(),
            embedding: Some(e2.clone()),
            ..Default::default()
        })
        .unwrap();
        let hits = s.search("", SearchMode::Vec, Some(&e1), 10).unwrap();
        assert_eq!(hits[0].text, "a");
    }

    #[test]
    fn hybrid_search() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut s = Store::open(tmp.path()).unwrap();
        s.put(&PutRequest {
            text: "rust memory sqlite".into(),
            embedding: Some(fake_emb(5)),
            ..Default::default()
        })
        .unwrap();
        s.put(&PutRequest {
            text: "python pandas".into(),
            embedding: Some(fake_emb(9)),
            ..Default::default()
        })
        .unwrap();
        let hits = s
            .search("rust", SearchMode::Hybrid, Some(&fake_emb(5)), 10)
            .unwrap();
        assert!(hits.iter().any(|h| h.text.contains("rust")));
    }

    #[cfg(feature = "encryption")]
    #[test]
    fn brain_key_derive_and_roundtrip() {
        let sig = b"abcdefghijklmnopqrstuvwxyz012345abcdefghijklmnopqrstuvwxyz012345";
        let hw = "AA:BB:CC:DD:EE:FF";
        let key = derive_brain_key(sig, hw);

        // Key must be deterministic
        let key2 = derive_brain_key(sig, hw);
        assert_eq!(key, key2);

        // Different hw_fp must yield different key
        let key_other = derive_brain_key(sig, "11:22:33:44:55:66");
        assert_ne!(key, key_other);

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_owned();

        // Write with correct key
        {
            let mut s = Store::open_with_brain_key(&path, &key).unwrap();
            s.put(&PutRequest {
                text: "brain key test document".into(),
                ..Default::default()
            })
            .unwrap();
        }

        // Reopen with same key — must find the document
        {
            let s = Store::open_with_brain_key(&path, &key).unwrap();
            let hits = s.search("brain key", SearchMode::Lex, None, 5).unwrap();
            assert_eq!(hits.len(), 1, "should find the stored doc on reopen");
        }

        // Reopen with wrong key — must fail
        {
            let result = Store::open_with_brain_key(&path, &key_other);
            assert!(
                result.is_err(),
                "wrong key must produce an error, not open successfully"
            );
        }
    }

    #[test]
    fn put_batch_fast_throughput() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut s = Store::open(tmp.path()).unwrap();
        let n = 10_000usize;
        let reqs: Vec<PutRequest> = (0..n)
            .map(|i| PutRequest {
                text: format!("fast ingest doc number {i} with some unique content for dedup"),
                title: Some(format!("doc-{i}")),
                embedding: None,
                ..Default::default()
            })
            .collect();
        let t0 = std::time::Instant::now();
        let ids = s.put_batch_fast(&reqs).unwrap();
        let elapsed = t0.elapsed();
        assert_eq!(ids.len(), n);
        let docs_per_sec = n as f64 / elapsed.as_secs_f64();
        eprintln!("put_batch_fast: {n} docs in {elapsed:?} = {docs_per_sec:.0} docs/sec");
        // tantivy-fts ON: tantivy batch write adds ~80ms/10k overhead → lower floor.
        // tantivy-fts OFF: FTS5-only, M4 Max yields 40-50k/s.
        #[cfg(feature = "tantivy-fts")]
        let floor = 5_000.0_f64;
        #[cfg(not(feature = "tantivy-fts"))]
        let floor = 30_000.0_f64;
        assert!(
            docs_per_sec > floor,
            "expected >{floor:.0} docs/sec, got {docs_per_sec:.0}"
        );
        // Verify FTS5 is usable immediately
        let hits = s
            .search("unique content", SearchMode::Lex, None, 5)
            .unwrap();
        assert!(!hits.is_empty());
        // Verify embedding rejection
        let bad = vec![PutRequest {
            text: "reject me".into(),
            embedding: Some(fake_emb(1)),
            ..Default::default()
        }];
        assert!(s.put_batch_fast(&bad).is_err());
    }

    // Throughput assertion is load-sensitive; run explicitly with `cargo test -- --ignored`
    #[test]
    #[ignore]
    fn put_batch_deferred_fts_throughput() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut s = Store::open(tmp.path()).unwrap();
        let n = 10_000usize;
        let reqs: Vec<PutRequest> = (0..n)
            .map(|i| PutRequest {
                text: format!("deferred fts ingest doc {i} with unique searchable content here"),
                title: Some(format!("deferred-{i}")),
                embedding: None,
                ..Default::default()
            })
            .collect();
        let t0 = std::time::Instant::now();
        let ids = s.put_batch_deferred_fts(&reqs).unwrap();
        let elapsed = t0.elapsed();
        assert_eq!(ids.len(), n);
        let docs_per_sec = n as f64 / elapsed.as_secs_f64();
        eprintln!("put_batch_deferred_fts: {n} docs in {elapsed:?} = {docs_per_sec:.0} docs/sec");
        // tantivy-fts ON: tantivy mirror adds overhead → lower floor.
        // tantivy-fts OFF: deferred FTS5 skip yields >80k/s on M4 Max.
        // Conservative floor — avoids false failures under CI/parallel load
        #[cfg(feature = "tantivy-fts")]
        let deferred_floor = 2_000.0_f64;
        #[cfg(not(feature = "tantivy-fts"))]
        let deferred_floor = 30_000.0_f64;
        assert!(
            docs_per_sec > deferred_floor,
            "expected >{deferred_floor:.0} docs/sec (Tier-2 target), got {docs_per_sec:.0}"
        );
        // Verify FTS5 is usable after deferred merge
        let hits = s
            .search("unique searchable content", SearchMode::Lex, None, 5)
            .unwrap();
        assert!(
            !hits.is_empty(),
            "FTS5 must be queryable after deferred merge"
        );
        // Verify trigger is restored — a normal put should also appear in FTS5
        let extra = PutRequest {
            text: "triggerrestoredcheck unique beacon text xyzzy".into(),
            title: Some("beacon".into()),
            embedding: None,
            ..Default::default()
        };
        s.put_batch_deferred_fts(&[extra]).unwrap();
        let beacon = s
            .search("triggerrestoredcheck", SearchMode::Lex, None, 1)
            .unwrap();
        assert!(!beacon.is_empty(), "trigger must be restored after batch");
        // Verify embedding rejection
        let bad = vec![PutRequest {
            text: "reject me".into(),
            embedding: Some(fake_emb(1)),
            ..Default::default()
        }];
        assert!(s.put_batch_deferred_fts(&bad).is_err());
    }

    /// Verify search_vec_exact returns all inserted docs when limit >= n,
    /// and that the turbo brute-force path is exercised (no Hamming skip).
    #[cfg(feature = "turbo")]
    #[test]
    fn exact_rerank_recall_guarantee() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut s = Store::open(tmp.path()).unwrap();
        let n = 100usize;
        // Insert n docs with distinct embeddings
        for i in 0..n {
            s.put(&PutRequest {
                text: format!("exact rerank doc {i}"),
                embedding: Some(fake_emb(i as u8 + 1)),
                ..Default::default()
            })
            .unwrap();
        }
        // Warm the turbo index
        let _ = s.search_vec(&fake_emb(1), 1).unwrap();

        // Query with seed=1 — exact top-10 must include the doc inserted with seed=1
        let exact_hits = s.search_vec_exact(&fake_emb(1), 10).unwrap();
        assert_eq!(exact_hits.len(), 10);
        // Top result must be the doc with seed=1 (identical embedding → distance≈0)
        assert!(
            exact_hits[0].text.contains("doc 0") || exact_hits[0].score >= exact_hits[1].score,
            "exact search must return highest-scoring doc first"
        );

        // Scores must be in descending order
        for w in exact_hits.windows(2) {
            assert!(
                w[0].score >= w[1].score,
                "scores must be non-increasing: {} < {}",
                w[0].score,
                w[1].score
            );
        }
    }

    /// Filter pushdown: 1000 docs, 50% category=A, 50% category=B.
    /// All returned hits must have category=A.
    /// Filtered recall@10 >= baseline (unfiltered ∩ A).
    #[test]
    fn metadata_filter_pushdown_recall() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut s = Store::open(tmp.path()).unwrap();
        let n = 1000usize;
        let mut category_a_ids = std::collections::HashSet::new();
        for i in 0..n {
            let cat = if i % 2 == 0 { "A" } else { "B" };
            let meta = serde_json::json!({ "category": cat });
            let id = s
                .put(&PutRequest {
                    text: format!("doc {i}"),
                    embedding: Some(fake_emb((i % 251) as u8)),
                    meta: Some(meta),
                    ..Default::default()
                })
                .unwrap();
            if i % 2 == 0 {
                category_a_ids.insert(id);
            }
        }

        let query_emb = fake_emb(1);
        let k = 10usize;

        // Baseline: no filter
        let base_hits = s.search_vec(&query_emb, k).unwrap();
        let unfiltered_a_count = base_hits
            .iter()
            .filter(|h| category_a_ids.contains(&h.id))
            .count();

        // Filtered search
        let opts = SearchOptions {
            filter: Some(MetadataPredicate::Eq {
                key: "category".into(),
                value: serde_json::json!("A"),
            }),
            ef_multiplier: None,
            ..Default::default()
        };
        let filtered_hits = s.search_vec_filtered(&query_emb, k, &opts).unwrap();

        // All hits must be category=A
        for hit in &filtered_hits {
            assert!(
                category_a_ids.contains(&hit.id),
                "hit {} is not category=A",
                hit.id
            );
        }

        // Must return K hits
        assert_eq!(filtered_hits.len(), k);

        // Filtered recall >= unfiltered recall (oversampling should find at least as many A docs)
        assert!(
            filtered_hits.len() >= unfiltered_a_count,
            "filtered recall={} < baseline recall={}",
            filtered_hits.len(),
            unfiltered_a_count
        );
    }

    /// Latency bench: filter on vs off — run with --nocapture to see timing.
    #[test]
    fn filter_bench_latency() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut s = Store::open(tmp.path()).unwrap();
        let n = 1000usize;
        for i in 0..n {
            let cat = if i % 2 == 0 { "A" } else { "B" };
            s.put(&PutRequest {
                text: format!("bench doc {i}"),
                embedding: Some(fake_emb((i % 251) as u8)),
                meta: Some(serde_json::json!({ "category": cat })),
                ..Default::default()
            })
            .unwrap();
        }
        let query_emb = fake_emb(42);
        let k = 10usize;
        let iters = 50usize;

        // Baseline (no filter)
        let t0 = std::time::Instant::now();
        for _ in 0..iters {
            s.search_vec(&query_emb, k).unwrap();
        }
        let base_us = t0.elapsed().as_micros() / iters as u128;

        // With filter
        let opts = SearchOptions {
            filter: Some(MetadataPredicate::Eq {
                key: "category".into(),
                value: serde_json::json!("A"),
            }),
            ef_multiplier: None,
            ..Default::default()
        };
        let t1 = std::time::Instant::now();
        for _ in 0..iters {
            s.search_vec_filtered(&query_emb, k, &opts).unwrap();
        }
        let filt_us = t1.elapsed().as_micros() / iters as u128;

        eprintln!(
            "filter_bench: base={base_us}µs filter={filt_us}µs overhead=+{}µs ({:.0}%)",
            filt_us.saturating_sub(base_us),
            if base_us > 0 {
                (filt_us as f64 / base_us as f64 - 1.0) * 100.0
            } else {
                0.0
            }
        );
        // Sanity: filter should not be more than 20× slower than no-filter
        assert!(
            filt_us < base_us * 20,
            "filter overhead too high: {filt_us}µs vs {base_us}µs base"
        );
    }

    /// Compound AND: category=A AND price<100
    #[test]
    fn compound_and_filter() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut s = Store::open(tmp.path()).unwrap();
        // 1000 docs: category alternates A/B, price = i
        for i in 0..1000usize {
            let cat = if i % 2 == 0 { "A" } else { "B" };
            s.put(&PutRequest {
                text: format!("doc {i}"),
                embedding: Some(fake_emb((i % 251) as u8)),
                meta: Some(serde_json::json!({ "category": cat, "price": i })),
                ..Default::default()
            })
            .unwrap();
        }
        let pred = MetadataPredicate::And(vec![
            MetadataPredicate::Eq {
                key: "category".into(),
                value: serde_json::json!("A"),
            },
            MetadataPredicate::Lt {
                key: "price".into(),
                value: 100.0,
            },
        ]);
        // Test matches() directly
        for i in 0..1000usize {
            let cat = if i % 2 == 0 { "A" } else { "B" };
            let meta = serde_json::json!({ "category": cat, "price": i });
            let expected = cat == "A" && i < 100;
            assert_eq!(pred.matches(Some(&meta)), expected, "i={i}");
        }
        // Test via search
        let opts = SearchOptions {
            filter: Some(pred),
            ..Default::default()
        };
        let hits = s.search_vec_filtered(&fake_emb(1), 20, &opts).unwrap();
        for h in &hits {
            let meta = s.get(h.id).unwrap().meta.unwrap();
            assert_eq!(meta["category"], "A");
            assert!(meta["price"].as_f64().unwrap() < 100.0);
        }
    }

    /// Compound OR: tag=foo OR tag=bar
    #[test]
    fn compound_or_filter() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut s = Store::open(tmp.path()).unwrap();
        let tags = ["foo", "bar", "baz", "qux"];
        for i in 0..1000usize {
            let tag = tags[i % 4];
            s.put(&PutRequest {
                text: format!("doc {i}"),
                embedding: Some(fake_emb((i % 251) as u8)),
                meta: Some(serde_json::json!({ "tag": tag })),
                ..Default::default()
            })
            .unwrap();
        }
        let pred = MetadataPredicate::Or(vec![
            MetadataPredicate::Eq {
                key: "tag".into(),
                value: serde_json::json!("foo"),
            },
            MetadataPredicate::Eq {
                key: "tag".into(),
                value: serde_json::json!("bar"),
            },
        ]);
        for i in 0..1000usize {
            let tag = tags[i % 4];
            let meta = serde_json::json!({ "tag": tag });
            let expected = tag == "foo" || tag == "bar";
            assert_eq!(pred.matches(Some(&meta)), expected, "i={i} tag={tag}");
        }
        let opts = SearchOptions {
            filter: Some(pred),
            ..Default::default()
        };
        let hits = s.search_vec_filtered(&fake_emb(1), 20, &opts).unwrap();
        for h in &hits {
            let meta = s.get(h.id).unwrap().meta.unwrap();
            let tag = meta["tag"].as_str().unwrap();
            assert!(tag == "foo" || tag == "bar", "unexpected tag={tag}");
        }
    }

    /// Range: price>=50 AND price<=200
    #[test]
    fn range_filter_gte_lte() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut s = Store::open(tmp.path()).unwrap();
        for i in 0..1000usize {
            s.put(&PutRequest {
                text: format!("doc {i}"),
                embedding: Some(fake_emb((i % 251) as u8)),
                meta: Some(serde_json::json!({ "price": i })),
                ..Default::default()
            })
            .unwrap();
        }
        let pred = MetadataPredicate::And(vec![
            MetadataPredicate::Gte {
                key: "price".into(),
                value: 50.0,
            },
            MetadataPredicate::Lte {
                key: "price".into(),
                value: 200.0,
            },
        ]);
        for i in 0..1000usize {
            let meta = serde_json::json!({ "price": i });
            assert_eq!(pred.matches(Some(&meta)), i >= 50 && i <= 200, "i={i}");
        }
        let opts = SearchOptions {
            filter: Some(pred),
            ..Default::default()
        };
        let hits = s.search_vec_filtered(&fake_emb(1), 20, &opts).unwrap();
        for h in &hits {
            let meta = s.get(h.id).unwrap().meta.unwrap();
            let p = meta["price"].as_f64().unwrap();
            assert!(p >= 50.0 && p <= 200.0, "price={p} out of range");
        }
    }

    /// NOT predicate
    #[test]
    fn not_filter() {
        let pred = MetadataPredicate::Not(Box::new(MetadataPredicate::Eq {
            key: "status".into(),
            value: serde_json::json!("deleted"),
        }));
        let active = serde_json::json!({ "status": "active" });
        let deleted = serde_json::json!({ "status": "deleted" });
        assert!(pred.matches(Some(&active)));
        assert!(!pred.matches(Some(&deleted)));
    }

    /// In predicate
    #[test]
    fn in_filter() {
        let pred = MetadataPredicate::In {
            key: "color".into(),
            values: vec![serde_json::json!("red"), serde_json::json!("blue")],
        };
        assert!(pred.matches(Some(&serde_json::json!({ "color": "red" }))));
        assert!(pred.matches(Some(&serde_json::json!({ "color": "blue" }))));
        assert!(!pred.matches(Some(&serde_json::json!({ "color": "green" }))));
    }

    #[test]
    #[ignore = "flaky under parallel-load; run explicit: cargo test bench_tantivy_warm_start_10k -- --include-ignored"]
    #[cfg(feature = "tantivy-fts")]
    fn bench_tantivy_warm_start_10k() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("brain.db");

        // Populate + commit tantivy index.
        {
            let mut s = Store::open(&db_path).unwrap();
            let reqs: Vec<PutRequest> = (0..10_000usize)
                .map(|i| PutRequest {
                    text: format!("warmstart bench doc {i} topic {} unique content", i % 20),
                    title: Some(format!("Doc {i}")),
                    embedding: None,
                    ..Default::default()
                })
                .collect();
            s.put_batch_fast(&reqs).unwrap();
            // Trigger commit + persist last_indexed_doc_id.
            let _ = s.search("warmstart bench", SearchMode::Lex, None, 5);
        }

        // Cold restart with warm-start-delta (should index 0 new docs).
        let t0 = std::time::Instant::now();
        let _ = Store::open(&db_path).unwrap();
        let warm_ms = t0.elapsed().as_millis();
        eprintln!("tantivy warm-start (10k persisted): {}ms", warm_ms);

        // Cold restart WITHOUT persist (index all 10k from scratch).
        let dir2 = tempfile::tempdir().unwrap();
        let db_path2 = dir2.path().join("brain2.db");
        {
            let mut s = Store::open(&db_path2).unwrap();
            let reqs: Vec<PutRequest> = (0..10_000usize)
                .map(|i| PutRequest {
                    text: format!("no-persist doc {i} topic {} unique", i % 20),
                    title: Some(format!("Doc {i}")),
                    embedding: None,
                    ..Default::default()
                })
                .collect();
            s.put_batch_fast(&reqs).unwrap();
            // No search — tantivy last_indexed_doc_id = 0.
        }
        let t1 = std::time::Instant::now();
        let _ = Store::open(&db_path2).unwrap();
        let cold_ms = t1.elapsed().as_millis();
        eprintln!(
            "tantivy cold-rebuild (10k docs, no prior persist): {}ms",
            cold_ms
        );

        assert!(
            warm_ms < 100,
            "warm-start must be <100ms, got {}ms",
            warm_ms
        );
        eprintln!(
            "speedup: cold={}ms warm={}ms ratio={:.1}×",
            cold_ms,
            warm_ms,
            if warm_ms > 0 {
                cold_ms as f64 / warm_ms as f64
            } else {
                f64::INFINITY
            }
        );
    }
}
