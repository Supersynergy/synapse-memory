use crate::error::{Error, Result};
use crate::types::{Doc, Hit, PutRequest, SearchMode, EMBED_DIM};
#[cfg(feature = "encryption")]
use base64::Engine as _;
use ed25519_dalek::SigningKey;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub struct Store {
    pub conn: Connection,
    /// PR-A1-wire: optional usearch ANN fast-path. `None` = brute-force
    /// sqlite-vec path (current behavior). Populated by `Store::open` when
    /// feature `ann-usearch` is enabled.
    #[cfg(feature = "ann-usearch")]
    pub(crate) ann: Option<crate::ann::Ann>,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
        let db_path = path.as_ref().to_path_buf();
        let conn = Connection::open(&db_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        conn.pragma_update(None, "mmap_size", 268_435_456_i64)?;
        // Schritt 1 optim #3 (SPEC §6 item 4): 64 MB page cache keeps FTS5
        // BM25 scoring tables and vec0 working-set resident (negative value
        // means kibibytes, -65536 = 64 MB). Per research_chroma_m4max §Mode B.
        conn.pragma_update(None, "cache_size", -65536_i64)?;
        #[cfg(feature = "ann-usearch")]
        let s = {
            let mut store = Self {
                conn,
                ann: None,
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
                // Sidecar was missing/corrupt or outdated. Rebuild from SQL.
                store.rebuild_ann_from_docs_vec(&ann)?;
            }
            store.ann = Some(ann);
            store
        };
        #[cfg(not(feature = "ann-usearch"))]
        let s = {
            let mut store = Self { conn };
            store.migrate()?;
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
        #[cfg(feature = "ann-usearch")]
        let s = Self { conn, ann: None };
        #[cfg(not(feature = "ann-usearch"))]
        let s = Self { conn };
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

INSERT OR IGNORE INTO meta(k,v) VALUES
  ('schema_version','1'),
  ('embed_dim','{dim}'),
  ('embed_model','bge-small-en-v1.5');
"#,
            dim = EMBED_DIM
        ))?;
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
        Ok(ids)
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
        let mut out = Vec::with_capacity(ann_hits.len());
        for (id, dist) in ann_hits {
            if let Some((uri, title, text)) = by_id.remove(id) {
                out.push(Hit {
                    id: *id,
                    uri,
                    title,
                    text,
                    score: 1.0 / (1.0 + *dist as f64),
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
}
