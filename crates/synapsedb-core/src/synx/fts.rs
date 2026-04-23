//! Tantivy-backed full-text search chunk — Phase 2 of `.synx`.
//!
//! Strategy: build an in-memory Tantivy index, snapshot its segment files into
//! a zstd-compressed `FtsSegment` chunk, and mount it read-only on open.
//!
//! This file compiles behind the `fts-tantivy` feature. When the feature is off
//! the public API degrades to a no-op wrapper so the rest of the crate still
//! builds.

#[cfg(feature = "fts-tantivy")]
pub use imp::*;
#[cfg(not(feature = "fts-tantivy"))]
pub use stub::*;

#[cfg(feature = "fts-tantivy")]
mod imp {
    use crate::error::{Error, Result};
    use tantivy::collector::TopDocs;
    use tantivy::query::QueryParser;
    use tantivy::schema::Value as SchemaValue;
    use tantivy::schema::{Schema, STORED, TEXT};
    use tantivy::{doc, Index, IndexReader, IndexWriter, ReloadPolicy};

    pub struct FtsIndex {
        index: Index,
        reader: IndexReader,
        schema: Schema,
    }

    impl FtsIndex {
        /// Build a fresh in-memory index.
        pub fn new() -> Result<Self> {
            let mut sb = Schema::builder();
            sb.add_text_field("id", STORED);
            sb.add_text_field("title", TEXT | STORED);
            sb.add_text_field("text", TEXT);
            sb.add_text_field("scope", STORED);
            let schema = sb.build();
            let index = Index::create_in_ram(schema.clone());
            let reader = index
                .reader_builder()
                .reload_policy(ReloadPolicy::Manual)
                .try_into()
                .map_err(|e| Error::Format(format!("tantivy reader: {e}")))?;
            Ok(Self {
                index,
                reader,
                schema,
            })
        }

        pub fn write(&self, rows: &[(String, String, String, String)]) -> Result<()> {
            let mut w: IndexWriter = self
                .index
                .writer(50_000_000)
                .map_err(|e| Error::Format(format!("tantivy writer: {e}")))?;
            let id_f = self.schema.get_field("id").unwrap();
            let ti_f = self.schema.get_field("title").unwrap();
            let tx_f = self.schema.get_field("text").unwrap();
            let sc_f = self.schema.get_field("scope").unwrap();
            for (id, title, text, scope) in rows {
                w.add_document(doc!(
                    id_f => id.as_str(),
                    ti_f => title.as_str(),
                    tx_f => text.as_str(),
                    sc_f => scope.as_str(),
                ))
                .map_err(|e| Error::Format(format!("tantivy add: {e}")))?;
            }
            w.commit()
                .map_err(|e| Error::Format(format!("tantivy commit: {e}")))?;
            self.reader
                .reload()
                .map_err(|e| Error::Format(format!("tantivy reload: {e}")))?;
            Ok(())
        }

        pub fn search(&self, q: &str, limit: usize) -> Result<Vec<(f32, String, String)>> {
            let searcher = self.reader.searcher();
            let title = self.schema.get_field("title").unwrap();
            let text = self.schema.get_field("text").unwrap();
            let id = self.schema.get_field("id").unwrap();
            let parser = QueryParser::for_index(&self.index, vec![title, text]);
            let query = parser
                .parse_query(q)
                .map_err(|e| Error::Format(format!("tantivy parse: {e}")))?;
            let hits = searcher
                .search(&query, &TopDocs::with_limit(limit))
                .map_err(|e| Error::Format(format!("tantivy search: {e}")))?;
            let mut out = Vec::with_capacity(hits.len());
            for (score, addr) in hits {
                let doc: tantivy::TantivyDocument = searcher
                    .doc(addr)
                    .map_err(|e| Error::Format(format!("tantivy doc: {e}")))?;
                let doc_id = doc
                    .get_first(id)
                    .and_then(|v| SchemaValue::as_str(&v))
                    .unwrap_or("")
                    .to_string();
                let title_s = doc
                    .get_first(title)
                    .and_then(|v| SchemaValue::as_str(&v))
                    .unwrap_or("")
                    .to_string();
                out.push((score, doc_id, title_s));
            }
            Ok(out)
        }
    }
}

#[cfg(not(feature = "fts-tantivy"))]
mod stub {
    use crate::error::Result;
    pub struct FtsIndex;
    impl FtsIndex {
        pub fn new() -> Result<Self> {
            Ok(Self)
        }
        pub fn write(&self, _rows: &[(String, String, String, String)]) -> Result<()> {
            Ok(())
        }
        pub fn search(&self, _q: &str, _limit: usize) -> Result<Vec<(f32, String, String)>> {
            Ok(Vec::new())
        }
    }
}

#[cfg(all(test, feature = "fts-tantivy"))]
mod tests {
    use super::imp::*;

    #[test]
    fn tantivy_index_and_search() {
        let idx = FtsIndex::new().unwrap();
        idx.write(&[
            (
                "a".into(),
                "Rust ships".into(),
                "rust ships here tonight".into(),
                "global".into(),
            ),
            (
                "b".into(),
                "Python docs".into(),
                "docs about python typing".into(),
                "global".into(),
            ),
            (
                "c".into(),
                "Rust memory".into(),
                "memory allocator in rust".into(),
                "global".into(),
            ),
        ])
        .unwrap();
        let hits = idx.search("rust", 10).unwrap();
        assert!(hits.len() >= 2);
        let ids: Vec<_> = hits.iter().map(|h| h.1.clone()).collect();
        assert!(ids.contains(&"a".to_string()));
        assert!(ids.contains(&"c".to_string()));
    }
}
