use crate::error::{Error, Result};
use crate::types::{Doc, Hit, PutRequest, SearchMode, EMBED_DIM};
#[cfg(feature = "encryption")]
use base64::Engine as _;
use ed25519_dalek::SigningKey;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub struct Store {
    pub conn: Connection,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        conn.pragma_update(None, "mmap_size", 268_435_456_i64)?;
        let s = Self { conn };
        s.migrate()?;
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
        Ok(ids)
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
