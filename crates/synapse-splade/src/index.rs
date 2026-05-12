//! SPLADE inverted index — SQLite-backed posting lists.
//! Schema: `postings(term_id INTEGER, doc_id INTEGER, weight REAL)`
//! Score(q, d) = Σ_{t ∈ q∩d} q_weight(t) * d_weight(t)   [dot-product over sparse vecs]

use anyhow::Result;
use rusqlite::{params, Connection};
use std::collections::HashMap;

use crate::SparseVec;

pub struct SpladeIndex {
    conn: Connection,
}

impl SpladeIndex {
    /// Open (or create) a SQLite-backed SPLADE index at `path`.
    /// Use `":memory:"` for ephemeral / test usage.
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            CREATE TABLE IF NOT EXISTS postings (
                term_id INTEGER NOT NULL,
                doc_id  INTEGER NOT NULL,
                weight  REAL    NOT NULL,
                PRIMARY KEY (term_id, doc_id)
            );
            CREATE INDEX IF NOT EXISTS idx_postings_term ON postings(term_id);
        ",
        )?;
        Ok(Self { conn })
    }

    /// Add (or replace) a document's sparse representation.
    pub fn add_doc(&mut self, doc_id: u64, sparse: &SparseVec) -> Result<()> {
        let tx = self.conn.transaction()?;
        // Remove old postings for this doc (upsert via DELETE + INSERT)
        tx.execute(
            "DELETE FROM postings WHERE doc_id = ?1",
            params![doc_id as i64],
        )?;
        for (&term_id, &weight) in sparse {
            tx.execute(
                "INSERT INTO postings(term_id, doc_id, weight) VALUES(?1, ?2, ?3)",
                params![term_id as i64, doc_id as i64, weight as f64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Search: dot-product score over shared terms, return top-k (doc_id, score).
    pub fn search(&self, query: &SparseVec, top_k: usize) -> Result<Vec<(u64, f32)>> {
        if query.is_empty() || top_k == 0 {
            return Ok(vec![]);
        }

        // Build score accumulator: doc_id → score
        let mut scores: HashMap<i64, f32> = HashMap::new();

        for (&term_id, &q_weight) in query {
            let mut stmt = self
                .conn
                .prepare_cached("SELECT doc_id, weight FROM postings WHERE term_id = ?1")?;
            let rows = stmt.query_map(params![term_id as i64], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)? as f32))
            })?;
            for row in rows {
                let (doc_id, d_weight) = row?;
                *scores.entry(doc_id).or_insert(0.0) += q_weight * d_weight;
            }
        }

        // Sort descending, take top_k
        let mut ranked: Vec<(u64, f32)> =
            scores.into_iter().map(|(did, s)| (did as u64, s)).collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        ranked.truncate(top_k);
        Ok(ranked)
    }

    /// Number of indexed documents (approximate — counts distinct doc_ids).
    pub fn doc_count(&self) -> Result<u64> {
        let n: i64 =
            self.conn
                .query_row("SELECT COUNT(DISTINCT doc_id) FROM postings", [], |r| {
                    r.get(0)
                })?;
        Ok(n as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SpladeEncoder;

    fn make_index() -> SpladeIndex {
        SpladeIndex::open(":memory:").unwrap()
    }

    #[test]
    fn smoke_10_docs() {
        let enc = SpladeEncoder::default();
        let mut idx = make_index();

        let docs = vec![
            (0u64, "splade neural sparse retrieval model"),
            (1, "dense retrieval bi-encoder sentence"),
            (2, "inverted index posting list BM25"),
            (3, "transformer masked language model BERT"),
            (4, "splade expansion vocabulary terms"),
            (5, "colbert late interaction multi-vector"),
            (6, "sparse representation regularisation"),
            (7, "neural ranking passage reranking"),
            (8, "query expansion pseudo relevance feedback"),
            (9, "MTEB benchmark retrieval recall"),
        ];

        for (id, text) in &docs {
            let sv = enc.encode(text).unwrap();
            idx.add_doc(*id, &sv).unwrap();
        }

        assert_eq!(idx.doc_count().unwrap(), 10);

        // Use doc 0's own sparse vec as query — guarantees overlap → non-empty results.
        // (Real SPLADE: query encoder produces terms that match doc terms by shared vocab)
        let q = enc.encode(docs[0].1).unwrap();
        let results = idx.search(&q, 5).unwrap();

        assert!(!results.is_empty(), "no results returned");
        // Doc 0 searched with its own vec must be top result (perfect self-match)
        assert_eq!(results[0].0, 0, "self-match should rank first");
        // Scores must be positive and decreasing
        let scores: Vec<f32> = results.iter().map(|(_, s)| *s).collect();
        assert!(scores[0] > 0.0);
        for w in scores.windows(2) {
            assert!(w[0] >= w[1], "scores not sorted: {:?}", scores);
        }
    }

    #[test]
    fn empty_query_returns_empty() {
        let mut idx = make_index();
        let enc = SpladeEncoder::default();
        idx.add_doc(0, &enc.encode("hello world").unwrap()).unwrap();
        let res = idx.search(&HashMap::new(), 10).unwrap();
        assert!(res.is_empty());
    }

    #[test]
    fn upsert_replaces_doc() {
        let enc = SpladeEncoder::default();
        let mut idx = make_index();
        let sv1 = enc.encode("original text").unwrap();
        let sv2 = enc.encode("completely different content xyz").unwrap();
        idx.add_doc(42, &sv1).unwrap();
        idx.add_doc(42, &sv2).unwrap();
        assert_eq!(idx.doc_count().unwrap(), 1);
        // Search with sv2 terms should find doc 42
        let res = idx.search(&sv2, 1).unwrap();
        assert_eq!(res[0].0, 42);
    }
}
