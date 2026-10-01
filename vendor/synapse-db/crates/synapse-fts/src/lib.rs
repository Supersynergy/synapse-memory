use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{FAST, NumericOptions, STORED, Schema, TextFieldIndexing, TextOptions};
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, doc};

const META_FILE: &str = "synapse_meta.json";

pub type FtsHit = (u64, f32);
pub type FtsResults = Vec<FtsHit>;

pub struct FtsIndex {
    index: Index,
    writer: IndexWriter,
    reader: IndexReader,
    schema: FtsSchema,
    /// None for in-RAM indices.
    index_path: Option<PathBuf>,
    /// Docs staged via `add()` but not yet committed. Lets `commit()`
    /// skip the segment flush entirely when nothing changed — read-path
    /// callers (search) then pay only a cheap `reader.reload()`.
    pending: usize,
}

struct FtsSchema {
    #[allow(dead_code)]
    schema: Schema,
    doc_id: tantivy::schema::Field,
    text: tantivy::schema::Field,
}

impl FtsIndex {
    pub fn new(path: &Path) -> Result<Self> {
        let mut schema_builder = Schema::builder();
        let doc_id =
            schema_builder.add_u64_field("doc_id", NumericOptions::default() | STORED | FAST);
        let text = schema_builder.add_text_field(
            "text",
            TextOptions::default().set_indexing_options(
                TextFieldIndexing::default()
                    .set_tokenizer("en_stem")
                    .set_index_option(tantivy::schema::IndexRecordOption::WithFreqsAndPositions),
            ),
        );
        let schema = schema_builder.build();

        let (index, index_path) =
            if path == Path::new(":memory:") || path.to_string_lossy().contains(":memory:") {
                (Index::create_in_ram(schema.clone()), None)
            } else {
                std::fs::create_dir_all(path)?;
                let idx = Index::create_in_dir(path, schema.clone())
                    .or_else(|_| Index::open_in_dir(path))
                    .context("open/create tantivy index")?;
                (idx, Some(path.to_path_buf()))
            };

        let writer = index.writer(50_000_000)?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;
        Ok(Self {
            index,
            writer,
            reader,
            schema: FtsSchema {
                schema,
                doc_id,
                text,
            },
            index_path,
            pending: 0,
        })
    }

    /// Stage a doc. Stays invisible to `search()` until `commit()` —
    /// batching `add()` calls without an intermediate commit is the
    /// intended fast ingest path.
    pub fn add(&mut self, doc_id: u64, text: &str) -> Result<()> {
        self.writer.add_document(doc!(
            self.schema.doc_id => doc_id,
            self.schema.text  => text
        ))?;
        self.pending += 1;
        Ok(())
    }

    /// Flush staged docs into a segment — only when `pending > 0` —
    /// then reload the reader so they become searchable.
    ///
    /// Safe on the read path: with nothing staged this degenerates to a
    /// `reader.reload()` (no segment build, no merge pressure, no fsync
    /// of `meta.json`). Callers that only need the latest *committed*
    /// state should prefer [`Self::reload`] instead.
    pub fn commit(&mut self) -> Result<()> {
        if self.pending > 0 {
            self.writer.commit()?;
            self.pending = 0;
        }
        self.reader.reload()?;
        Ok(())
    }

    /// Refresh the reader to see already-committed segments. Cheap —
    /// a no-op when the index generation hasn't changed. Staged
    /// (uncommitted) docs stay invisible.
    pub fn reload(&self) -> Result<()> {
        self.reader.reload()?;
        Ok(())
    }

    /// True when staged adds are waiting — i.e. `commit()` would
    /// actually flush a segment rather than only reload the reader.
    pub fn has_pending(&self) -> bool {
        self.pending > 0
    }

    /// Persist the highest doc_id that has been indexed into a meta file
    /// alongside the tantivy index directory. No-op for in-RAM indices.
    pub fn set_last_indexed_doc_id(&self, doc_id: u64) -> Result<()> {
        if let Some(ref p) = self.index_path {
            let meta_path = p.join(META_FILE);
            let json = format!("{{\"last_indexed_doc_id\":{}}}", doc_id);
            std::fs::write(&meta_path, json)?;
        }
        Ok(())
    }

    /// Read the last persisted doc_id. Returns 0 if not yet written.
    pub fn last_indexed_doc_id(&self) -> u64 {
        let Some(ref p) = self.index_path else {
            return 0;
        };
        let meta_path = p.join(META_FILE);
        let Ok(bytes) = std::fs::read(&meta_path) else {
            return 0;
        };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return 0;
        };
        v["last_indexed_doc_id"].as_u64().unwrap_or(0)
    }

    /// Returns `(doc_id, bm25_score)` sorted by descending score.
    pub fn search(&self, query: &str, top_k: usize) -> Result<FtsResults> {
        let searcher = self.reader.searcher();
        let parser = QueryParser::for_index(&self.index, vec![self.schema.text]);
        let q = parser.parse_query(query)?;
        let top_docs = searcher.search(&q, &TopDocs::with_limit(top_k))?;
        let mut out = Vec::with_capacity(top_docs.len());
        for (score, addr) in top_docs {
            let doc: TantivyDocument = searcher.doc(addr)?;
            if let Some(tantivy::schema::OwnedValue::U64(id)) =
                doc.get_first(self.schema.doc_id).cloned()
            {
                out.push((id, score));
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ram_index() -> FtsIndex {
        FtsIndex::new(Path::new(":memory:")).unwrap()
    }

    #[test]
    fn test_100_docs_bm25_ranking() {
        let mut idx = make_ram_index();
        for i in 0u64..100 {
            let text = if i == 42 {
                "foo bar foo bar foo bar baz".to_string()
            } else if i % 10 == 0 {
                format!("foo noise{i} text")
            } else {
                format!("random document content number {i}")
            };
            idx.add(i, &text).unwrap();
        }
        idx.commit().unwrap();

        let results = idx.search("foo bar", 10).unwrap();
        assert!(!results.is_empty(), "expected results");
        assert_eq!(results[0].0, 42, "doc 42 should rank first");
        for (_, score) in &results {
            assert!(*score > 0.0);
        }
    }

    #[test]
    fn test_empty_query_returns_empty() {
        let mut idx = make_ram_index();
        idx.add(1, "hello world").unwrap();
        idx.commit().unwrap();
        let r = idx.search("zzznomatchzzz", 5).unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn test_commit_skips_flush_when_nothing_pending() {
        let mut idx = make_ram_index();
        // No staged docs → commit() is a reload-only no-op.
        assert!(!idx.has_pending());
        idx.commit().unwrap();
        idx.reload().unwrap();

        idx.add(1, "hello deferred world").unwrap();
        assert!(idx.has_pending());
        // Staged doc is NOT visible before commit — staged writes are
        // never flushed by the read path.
        assert!(idx.search("deferred", 5).unwrap().is_empty());

        idx.commit().unwrap();
        assert!(!idx.has_pending());
        let r = idx.search("deferred", 5).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].0, 1);
        // A second commit() with nothing staged stays flush-free.
        idx.commit().unwrap();
        assert_eq!(idx.search("deferred", 5).unwrap().len(), 1);
    }

    #[test]
    fn test_last_indexed_doc_id_persistent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tantivy");
        let mut idx = FtsIndex::new(&path).unwrap();
        assert_eq!(idx.last_indexed_doc_id(), 0);
        idx.add(5, "hello persistent").unwrap();
        idx.commit().unwrap();
        idx.set_last_indexed_doc_id(5).unwrap();
        drop(idx);

        // Reopen — should read back 5.
        let idx2 = FtsIndex::new(&path).unwrap();
        assert_eq!(idx2.last_indexed_doc_id(), 5);
    }
}
