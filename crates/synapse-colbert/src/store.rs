//! ColBERT multi-vector SQLite storage + reranker.
//! Table: colbert_vecs(doc_id INTEGER, vecs BLOB)
//! vecs blob = zstd( json: Vec<Vec<f32>> )

use anyhow::Result;
use rusqlite::{Connection, params};
use crate::{ColbertEmbedder, max_sim};

pub struct ColbertStore<'a> {
    conn: &'a Connection,
    emb: ColbertEmbedder,
}

impl<'a> ColbertStore<'a> {
    pub fn new(conn: &'a Connection) -> Result<Self> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS colbert_vecs (
                doc_id  INTEGER PRIMARY KEY,
                vecs    BLOB NOT NULL
            );"
        )?;
        Ok(Self { conn, emb: ColbertEmbedder::default() })
    }

    /// Store pre-computed token vecs for a doc.
    pub fn add_colbert(&self, doc_id: i64, vecs: Vec<Vec<f32>>) -> Result<()> {
        let json = serde_json::to_vec(&vecs)?;
        let compressed = zstd::encode_all(json.as_slice(), 3)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO colbert_vecs (doc_id, vecs) VALUES (?1, ?2)",
            params![doc_id, compressed],
        )?;
        Ok(())
    }

    /// Embed text and store.
    pub fn embed_and_add(&self, doc_id: i64, text: &str) -> Result<()> {
        let vecs = self.emb.embed_doc(text)?;
        self.add_colbert(doc_id, vecs)
    }

    /// Rerank ANN candidates by ColBERT max-sim. Returns sorted (doc_id, score) desc.
    pub fn colbert_rerank(&self, query_text: &str, candidates: &[i64]) -> Result<Vec<(i64, f32)>> {
        let query_vecs = self.emb.embed_query(query_text)?;
        let mut scores: Vec<(i64, f32)> = candidates.iter().filter_map(|&doc_id| {
            let doc_vecs = self.load_vecs(doc_id).ok()?;
            let score = max_sim(&query_vecs, &doc_vecs);
            Some((doc_id, score))
        }).collect();
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(scores)
    }

    fn load_vecs(&self, doc_id: i64) -> Result<Vec<Vec<f32>>> {
        let compressed: Vec<u8> = self.conn.query_row(
            "SELECT vecs FROM colbert_vecs WHERE doc_id = ?1",
            params![doc_id],
            |row| row.get(0),
        )?;
        let json = zstd::decode_all(compressed.as_slice())?;
        Ok(serde_json::from_slice(&json)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn smoke_10_docs() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        let store = ColbertStore::new(&conn)?;

        let docs = [
            "the quick brown fox jumps over the lazy dog",
            "neural information retrieval with dense representations",
            "ColBERT late interaction multi vector search",
            "apple banana cherry date elderberry fig",
            "tokio async rust runtime performance benchmark",
            "matrix multiplication linear algebra operations",
            "deep learning transformer attention mechanism",
            "database indexing b-tree hash join query",
            "the cat sat on the mat",
            "late interaction reranking colbert max sim score",
        ];

        for (i, doc) in docs.iter().enumerate() {
            store.embed_and_add(i as i64, doc)?;
        }

        let query = "ColBERT late interaction reranking";
        let candidates: Vec<i64> = (0..10).collect();
        let ranked = store.colbert_rerank(query, &candidates)?;

        assert_eq!(ranked.len(), 10);
        // doc 2 ("ColBERT late interaction...") and doc 9 ("late interaction reranking colbert...")
        // should rank in top-3
        let top3_ids: Vec<i64> = ranked[..3].iter().map(|(id, _)| *id).collect();
        assert!(
            top3_ids.contains(&2) || top3_ids.contains(&9),
            "Expected colbert-related docs in top-3, got: {:?}", ranked
        );
        // scores descending
        for w in ranked.windows(2) {
            assert!(w[0].1 >= w[1].1, "scores not sorted: {:?}", ranked);
        }

        tracing::info!("ColBERT smoke top-3: {:?}", &ranked[..3]);
        Ok(())
    }
}
