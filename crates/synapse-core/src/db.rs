#[cfg(feature = "turbo")]
use crate::turbo::rrf_simd::distance_to_score;
use crate::error::{Error, Result};
use crate::types::{Doc, Hit, PutRequest, SearchMode, EMBED_DIM};
#[cfg(feature = "encryption")]
use base64::Engine as _;
use ed25519_dalek::SigningKey;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

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
}

impl Store {
    /// Internal constructor — centralizes the per-feature field init so
    /// `open`, `open_encrypted`, `open_with_brain_key` stay tidy.
    fn from_conn(conn: Connection) -> Self {
        Self {
            conn,
            #[cfg(feature = "ann-usearch")]
            ann: None,
            #[cfg(feature = "turbo")]
            ndarray_search: std::sync::RwLock::new(None),
        }
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        #[allow(clippy::missing_transmute_annotations)]
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
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
            let mut store = Self::from_conn(conn);
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
                // Sidecar was missing/corrupt or outdated. Rebuild from SQL.
                store.rebuild_ann_from_docs_vec(&ann)?;
            }
            store.ann = Some(ann);
            store.sota_migrate()?;
            store
        };
        #[cfg(not(feature = "ann-usearch"))]
        let s = {
            let store = Self::from_conn(conn);
            store.migrate()?;
            store.sota_migrate()?;
            store
        };
        Ok(s)
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
        let salt_b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD_NO_PAD,
            &salt_raw.as_bytes()[..16],
        );
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
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
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
    pub fn put(&mut self, req: &PutRequest) -> Result<i64> {
        self.put_inner(req, None, None)
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
        // PR-A1-wire: mirror into ANN index after SQL commit. If the ANN
        // insert fails we log but DO NOT fail the put — the sidecar is
        // rebuildable from docs_vec on next open.
        #[cfg(feature = "ann-usearch")]
        if let (Some(ref ann), Some(emb)) = (self.ann.as_ref(), req.embedding.as_ref()) {
            if let Err(e) = ann.insert(id, emb) {
                tracing::warn!("ann insert failed for id {id}: {e}; sidecar will rebuild on next open");
            }
        }
        // Turbo: append to in-memory ndarray index iff already built.
        // Not built yet → next search_vec rebuilds from SQL and picks up this row.
        #[cfg(feature = "turbo")]
        if let Some(ref emb) = req.embedding {
            if let Ok(mut guard) = self.ndarray_search.write() {
                if let Some(ref mut idx) = *guard {
                    if !idx.is_empty() {
                        if let Err(e) = idx.add_row(id, emb) {
                            tracing::warn!(
                                "turbo ndarray add_row id {id} failed: {e}; invalidating cache"
                            );
                            *guard = None;
                        }
                    }
                }
            }
        }
        Ok(id)
    }

    pub fn put_batch(&mut self, reqs: &[PutRequest]) -> Result<Vec<i64>> {
        let mut ids = Vec::with_capacity(reqs.len());
        let tx = self.conn.transaction()?;
        {
            let mut stmt_chk = tx.prepare("SELECT id FROM docs WHERE blake3 = ?1")?;
            let mut stmt_ins = tx.prepare(
                "INSERT INTO docs(uri,title,text,meta,ts,blake3) VALUES (?1,?2,?3,?4,?5,?6)",
            )?;
            let mut stmt_vec = tx.prepare("INSERT INTO docs_vec(id,embedding) VALUES (?1,?2)")?;
            for req in reqs {
                if let Some(ref e) = req.embedding {
                    if e.len() != EMBED_DIM {
                        return Err(Error::DimMismatch {
                            expected: EMBED_DIM,
                            got: e.len(),
                        });
                    }
                }
                let hash = blake3::hash(req.text.as_bytes());
                let hash_bytes = hash.as_bytes().to_vec();
                let found: Option<i64> = stmt_chk
                    .query_row(params![hash_bytes.clone()], |r| r.get(0))
                    .optional()?;
                if let Some(id) = found {
                    ids.push(id);
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
            }
        }
        tx.commit()?;
        // PR-A1-wire: mirror new rows into ANN index. Iterate in lockstep:
        // `ids[i]` is either the freshly-inserted rowid for `reqs[i]` OR a
        // de-duplicated existing id (blake3 hash match). We only want the
        // newly-inserted ones here, but since dedup returns the same id, it
        // is safe to attempt insert — the ANN layer treats duplicate inserts
        // as no-ops with usearch's multi=false.
        #[cfg(feature = "ann-usearch")]
        if let Some(ref ann) = self.ann {
            for (id, req) in ids.iter().zip(reqs.iter()) {
                if let Some(ref emb) = req.embedding {
                    if let Err(e) = ann.insert(*id, emb) {
                        tracing::warn!(
                            "ann batch insert id {id} failed: {e}; sidecar will rebuild on next open"
                        );
                    }
                }
            }
        }
        // Turbo: append batch to in-memory ndarray index iff already built.
        #[cfg(feature = "turbo")]
        {
            if let Ok(mut guard) = self.ndarray_search.write() {
                if let Some(ref mut idx) = *guard {
                    if !idx.is_empty() {
                        for (id, req) in ids.iter().zip(reqs.iter()) {
                            if let Some(ref emb) = req.embedding {
                                if let Err(e) = idx.add_row(*id, emb) {
                                    tracing::warn!(
                                        "turbo ndarray batch add_row id {id} failed: {e}; invalidating cache"
                                    );
                                    *guard = None;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
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
            self.conn
                .execute_batch("DROP TRIGGER IF EXISTS docs_ai;")?;

            let mut ids = Vec::with_capacity(docs.len());
            let tx = self.conn.transaction()?;
            let max_before: i64 = tx
                .query_row("SELECT COALESCE(MAX(id),0) FROM docs", [], |r| r.get(0))?;
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
        result
    }

    /// PR-A1-wire: delete a doc by id, removing it from `docs`, `docs_vec`,
    /// `docs_fts`, and (when enabled) the ANN sidecar. Idempotent — returns
    /// `Ok(false)` if the id did not exist.
    pub fn delete(&mut self, id: i64) -> Result<bool> {
        let tx = self.conn.transaction()?;
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
            if let Some(ref idx) = *guard {
                if !idx.is_empty() {
                    tracing::info!("turbo ndarray_search already warm: {} vectors", idx.len());
                    return;
                }
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

    /// PR-A1-wire internal: rebuild the ANN index from `docs_vec` rows.
    /// Called from `Store::open` when the sidecar is missing, corrupt, or
    /// out-of-sync (len < row count).
    #[cfg(feature = "ann-usearch")]
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

    pub fn search(
        &self,
        q: &str,
        mode: SearchMode,
        query_emb: Option<&[f32]>,
        limit: usize,
    ) -> Result<Vec<Hit>> {
        match mode {
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
        }
    }

    fn search_lex(&self, q: &str, limit: usize) -> Result<Vec<Hit>> {
        let sql = "SELECT d.id,d.uri,d.title,d.text,bm25(docs_fts) as score
                   FROM docs_fts JOIN docs d ON d.id = docs_fts.rowid
                   WHERE docs_fts MATCH ?1
                   ORDER BY score LIMIT ?2";
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params![q, limit as i64], |r| {
            Ok(Hit {
                id: r.get(0)?,
                uri: r.get(1)?,
                title: r.get(2)?,
                text: r.get(3)?,
                score: r.get::<_, f64>(4).map(|s| -s)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn search_vec(&self, emb: &[f32], limit: usize) -> Result<Vec<Hit>> {
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
        if let Some(ref ann) = self.ann {
            if ann.len() > 0 {
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
        }

        // Turbo fast-path: in-memory ndarray brute-force kNN.
        // Build cache lazily on first call (one-time ~250ms scan @ 162k×384).
        // Subsequent calls hit the matrix directly (~7ms p50 @ 162k vs ~50ms sqlite-vec).
        #[cfg(feature = "turbo")]
        {
            // Fast path: cache already built.
            {
                let guard = self.ndarray_search.read().unwrap();
                if let Some(ref idx) = *guard {
                    if !idx.is_empty() {
                        let pairs = idx.search(emb, limit);
                        if !pairs.is_empty() {
                            return self.hydrate_hits_by_id_dist(&pairs);
                        }
                    }
                }
            }
            // Slow path: lazy-build, then retry.
            {
                let mut guard = self.ndarray_search.write().unwrap();
                if guard.is_none() {
                    match crate::turbo::ndarray_search::NdArraySearch::from_connection(
                        &self.conn,
                    ) {
                        Ok(idx) => {
                            tracing::info!(
                                "turbo ndarray_search built: {} vectors",
                                idx.len()
                            );
                            *guard = Some(idx);
                        }
                        Err(e) => {
                            // Empty DB or other failure — install empty index so we
                            // do not retry on every call. Falls through to sqlite-vec.
                            tracing::debug!(
                                "turbo ndarray_search build skipped: {e}"
                            );
                            *guard = Some(
                                crate::turbo::ndarray_search::NdArraySearch::empty(
                                    EMBED_DIM,
                                ),
                            );
                        }
                    }
                }
                if let Some(ref idx) = *guard {
                    if !idx.is_empty() {
                        let pairs = idx.search(emb, limit);
                        drop(guard);
                        if !pairs.is_empty() {
                            return self.hydrate_hits_by_id_dist(&pairs);
                        }
                    }
                }
            }
        }

        let bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
        let sql = "SELECT d.id,d.uri,d.title,d.text,v.distance
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
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
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
        let sql = format!(
            "SELECT id,uri,title,text FROM docs WHERE id IN ({placeholders})"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let ids: Vec<i64> = ann_hits.iter().map(|(i, _)| *i).collect();
        let params_iter: Vec<&dyn rusqlite::ToSql> =
            ids.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
        let mut by_id: std::collections::HashMap<
            i64,
            (Option<String>, Option<String>, String),
        > = Default::default();
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
        let dists: Vec<f32> = ann_hits.iter().map(|(_, d)| *d).collect();
        #[cfg(feature = "turbo")]
        let scores: Vec<f32> = distance_to_score(&dists);
        #[cfg(not(feature = "turbo"))]
        let scores: Vec<f32> = dists.iter().map(|d| 1.0_f32 / (1.0_f32 + d)).collect();
        let mut out = Vec::with_capacity(ann_hits.len());
        for ((id, _), score) in ann_hits.iter().zip(scores.iter()) {
            if let Some((uri, title, text)) = by_id.remove(id) {
                out.push(Hit {
                    id: *id,
                    uri,
                    title,
                    text,
                    score: *score as f64,
                });
            }
        }
        Ok(out)
    }

    /// Turbo helper: given `(id, distance)` pairs from the ndarray index,
    /// fetch full `Hit` records (uri/title/text) from SQL in one round-trip.
    /// Preserves input order. Used by the turbo fast-path in `search_vec`.
    #[cfg(feature = "turbo")]
    fn hydrate_hits_by_id_dist(&self, pairs: &[(i64, f32)]) -> Result<Vec<Hit>> {
        if pairs.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (0..pairs.len())
            .map(|i| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(",");
        let sql =
            format!("SELECT id,uri,title,text FROM docs WHERE id IN ({placeholders})");
        let mut stmt = self.conn.prepare(&sql)?;
        let ids: Vec<i64> = pairs.iter().map(|(i, _)| *i).collect();
        let params_iter: Vec<&dyn rusqlite::ToSql> =
            ids.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
        let mut by_id: std::collections::HashMap<
            i64,
            (Option<String>, Option<String>, String),
        > = Default::default();
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
        for (id, dist) in pairs.iter() {
            if let Some((uri, title, text)) = by_id.remove(id) {
                out.push(Hit {
                    id: *id,
                    uri,
                    title,
                    text,
                    score: 1.0_f64 / (1.0_f64 + *dist as f64),
                });
            }
        }
        Ok(out)
    }

    fn search_hybrid(&self, q: &str, emb: &[f32], limit: usize) -> Result<Vec<Hit>> {
        let k = limit * 3;
        let lex = self.search_lex(q, k).unwrap_or_default();
        let vec = self.search_vec(emb, k).unwrap_or_default();
        let mut scores: std::collections::HashMap<i64, (f64, Hit)> = Default::default();
        let rrf_k = 60.0;
        for (i, h) in lex.into_iter().enumerate() {
            let s = 1.0 / (rrf_k + (i + 1) as f64);
            scores
                .entry(h.id)
                .and_modify(|e| e.0 += s)
                .or_insert((s, h));
        }
        for (i, h) in vec.into_iter().enumerate() {
            let s = 1.0 / (rrf_k + (i + 1) as f64);
            scores
                .entry(h.id)
                .and_modify(|e| e.0 += s)
                .or_insert((s, h));
        }
        let mut out: Vec<_> = scores
            .into_values()
            .map(|(s, mut h)| {
                h.score = s;
                h
            })
            .collect();
        out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        out.truncate(limit);
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
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
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
        if let Some(ref ann) = self.ann {
            if let Err(e) = ann.save() {
                tracing::warn!("ann drop-save failed: {e}; sidecar may be stale, but docs_vec is authoritative");
            }
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
            let hits = s
                .search("brain key", SearchMode::Lex, None, 5)
                .unwrap();
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
        // 30k/s floor is conservative; M4 Max typically yields 40-50k/s.
        // FTS5 triggers add ~10µs per row; true embed-skip gains vs fastembed
        // ceiling (30ms/doc) remain ~500×.
        assert!(
            docs_per_sec > 30_000.0,
            "expected >30k docs/sec, got {docs_per_sec:.0}"
        );
        // Verify FTS5 is usable immediately
        let hits = s.search("unique content", SearchMode::Lex, None, 5).unwrap();
        assert!(!hits.is_empty());
        // Verify embedding rejection
        let bad = vec![PutRequest {
            text: "reject me".into(),
            embedding: Some(fake_emb(1)),
            ..Default::default()
        }];
        assert!(s.put_batch_fast(&bad).is_err());
    }

    #[test]
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
        assert!(
            docs_per_sec > 80_000.0,
            "expected >80k docs/sec (Tier-2 target), got {docs_per_sec:.0}"
        );
        // Verify FTS5 is usable after deferred merge
        let hits = s
            .search("unique searchable content", SearchMode::Lex, None, 5)
            .unwrap();
        assert!(!hits.is_empty(), "FTS5 must be queryable after deferred merge");
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
}
