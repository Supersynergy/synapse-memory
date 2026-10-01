//! v1 (SQLite `.db`) → v2 (`.synx`) export path.
//!
//! Takes any iterator of `(text, meta_json)` pairs from v1 and writes a
//! well-formed `.synx` file. The CLI wires this up against `Store::iter_docs`
//! once that API lands; here we keep the migrate path engine-agnostic so the
//! format module never has to depend on SQLite internals.

use crate::error::{Error, Result};
use std::path::Path;

use super::chunk::{ChunkKind, Codec};
use super::header::SynxFlags;
use super::writer::SynxWriter;

pub struct MigrateRow {
    pub id: String,
    pub uri: Option<String>,
    pub title: Option<String>,
    pub text: String,
    pub ts: i64,
    pub meta_json: Option<String>,
    /// Optional mem0-style scope tag (see `synx::Scope::as_tag`).
    pub scope: Option<String>,
}

/// Export an iterator of v1 rows into a new `.synx` file.
pub fn export_rows<I, P>(rows: I, out: P, docs_per_batch: usize) -> Result<u64>
where
    I: IntoIterator<Item = MigrateRow>,
    P: AsRef<Path>,
{
    let mut w = SynxWriter::create(out.as_ref(), SynxFlags::COMPRESSED)?;
    let mut count: u64 = 0;
    let mut row_buf: Vec<serde_json::Value> = Vec::with_capacity(docs_per_batch);

    for r in rows {
        row_buf.push(serde_json::json!({
            "id":    r.id,
            "uri":   r.uri,
            "title": r.title,
            "ts":    r.ts,
            "meta":  r.meta_json,
            "scope": r.scope,
        }));
        w.append(ChunkKind::TextBlob, Codec::Zstd, r.text.as_bytes())?;
        count += 1;
        if row_buf.len() >= docs_per_batch {
            let bytes = serde_json::to_vec(&row_buf).map_err(|e| Error::Format(e.to_string()))?;
            w.append(ChunkKind::RowBatch, Codec::Zstd, &bytes)?;
            row_buf.clear();
        }
    }

    if !row_buf.is_empty() {
        let bytes = serde_json::to_vec(&row_buf).map_err(|e| Error::Format(e.to_string()))?;
        w.append(ChunkKind::RowBatch, Codec::Zstd, &bytes)?;
    }

    w.finish()?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::super::reader::SynxReader;
    use super::*;

    #[test]
    fn migrate_small_batch() {
        let rows = (0..5).map(|i| MigrateRow {
            id: format!("id-{i}"),
            uri: None,
            title: Some(format!("doc {i}")),
            text: format!("body of document {i}"),
            ts: 0,
            meta_json: None,
            scope: Some("global".into()),
        });
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("m.synx");
        let n = export_rows(rows, &out, 3).unwrap();
        assert_eq!(n, 5);
        let r = SynxReader::open(&out).unwrap();
        // 5 text blobs + 2 row batches (3+2) = 7 chunks (+ manifest is written separately)
        assert!(r.manifest.chunks.len() >= 7);
    }
}
