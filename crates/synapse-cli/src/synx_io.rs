/// Universal import/export for synx CLI.
///
/// Supported formats
/// -----------------
/// Import (read-only source):
///   .csv / .tsv          — RFC 4180, tab-separated for .tsv
///   .jsonl / .ndjson     — one JSON object per line {text, title?, uri?, meta?}
///   .json                — array of objects OR single object
///   .db / .sqlite / .sqlite3 — SQLite with a `docs` table compatible with Synapse schema
///   .synx / .brainpack   — Synapse portable pack (via snap::import)
///
/// Export (write):
///   .synx / .brainpack   — Synapse portable pack (via snap::export), lossless
///   .csv                 — title,uri,text,ts
///   .jsonl               — one JSON object per line
///
/// Optional (feature = "import-parquet"):
///   .parquet             — columnar Parquet; columns: text (required), title, uri, meta (JSON string)
///
/// Scaffold (no runtime dep, documented stub):
///   .lance               — LanceDB: use `synapse-migrate lancedb:///path/table --to out.db`
///   .qdrant              — Qdrant snapshot: use `synapse-migrate qdrant://host/col --to out.db`
use anyhow::{Context, Result, bail};
use std::io::{BufRead, Write};
use std::path::Path;
use synapse_core::{PutRequest, Store, snap};

type SqliteDocRow = (Option<String>, Option<String>, String, Option<String>);
type ExportDocRow = (
    i64,
    Option<String>,
    Option<String>,
    String,
    Option<String>,
    i64,
);

// ── Format detection ──────────────────────────────────────────────────────────

pub enum Format {
    Synx,
    Sqlite,
    Csv,
    Tsv,
    Jsonl,
    Json,
    #[cfg(feature = "import-parquet")]
    Parquet,
    Lance,
    Qdrant,
}

pub fn detect(path: &Path, forced: Option<&str>) -> Result<Format> {
    let name = if let Some(f) = forced {
        f.to_ascii_lowercase()
    } else {
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    match name.as_str() {
        "synx" | "brainpack" | "bp" => Ok(Format::Synx),
        "db" | "sqlite" | "sqlite3" => Ok(Format::Sqlite),
        "csv" => Ok(Format::Csv),
        "tsv" => Ok(Format::Tsv),
        "jsonl" | "ndjson" => Ok(Format::Jsonl),
        "json" => Ok(Format::Json),
        #[cfg(feature = "import-parquet")]
        "parquet" => Ok(Format::Parquet),
        #[cfg(not(feature = "import-parquet"))]
        "parquet" => bail!("Parquet support requires --features import-parquet"),
        "lance" => Ok(Format::Lance),
        "qdrant" => Ok(Format::Qdrant),
        other => bail!("unknown format: {other:?} — use --format to override"),
    }
}

// ── Import ────────────────────────────────────────────────────────────────────

/// Import `src` into `store`. Returns number of docs inserted.
pub fn import(src: &Path, store: &mut Store, forced: Option<&str>) -> Result<u64> {
    match detect(src, forced)? {
        Format::Synx => {
            let db_path = std::path::PathBuf::from(store.conn.path().context("store has no path")?);
            snap::import(src, &db_path)?;
            let count: i64 = store
                .conn
                .query_row("SELECT COUNT(*) FROM docs", [], |r| r.get(0))?;
            Ok(count as u64)
        }
        Format::Sqlite => import_sqlite(src, store),
        Format::Csv => import_dsv(src, store, b','),
        Format::Tsv => import_dsv(src, store, b'\t'),
        Format::Jsonl => import_jsonl(src, store),
        Format::Json => import_json(src, store),
        #[cfg(feature = "import-parquet")]
        Format::Parquet => import_parquet(src, store),
        Format::Lance => {
            bail!(
                "LanceDB import not available in synx.\n\
                 Use: synapse-migrate lancedb:///path/table_name --to brain.db"
            )
        }
        Format::Qdrant => {
            bail!(
                "Qdrant snapshot import not available in synx.\n\
                 Use: synapse-migrate qdrant://host:6334/collection --to brain.db"
            )
        }
    }
}

fn import_sqlite(src: &Path, store: &mut Store) -> Result<u64> {
    let src_conn = rusqlite::Connection::open(src)?;
    // Accept Synapse `docs` table layout; unknown schemas mapped to text fallback.
    let col_names: Vec<String> = {
        let mut s = src_conn.prepare("PRAGMA table_info(docs)")?;
        let rows: rusqlite::Result<Vec<String>> =
            s.query_map([], |r| r.get::<_, String>(1))?.collect();
        rows?
    };
    let has_title = col_names.iter().any(|c| c == "title");
    let has_uri = col_names.iter().any(|c| c == "uri");
    let has_meta = col_names.iter().any(|c| c == "meta");

    let sql = format!(
        "SELECT {}, {}, text, {} FROM docs",
        if has_title { "title" } else { "NULL" },
        if has_uri { "uri" } else { "NULL" },
        if has_meta { "meta" } else { "NULL" },
    );
    let mut stmt = src_conn.prepare(&sql)?;
    let rows: Vec<SqliteDocRow> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let total = rows.len() as u64;
    for (title, uri, text, meta_str) in rows {
        let meta = meta_str.and_then(|s| serde_json::from_str(&s).ok());
        store.put(&PutRequest {
            title,
            uri,
            text,
            meta,
            embedding: None,
        })?;
    }
    Ok(total)
}

fn import_dsv(src: &Path, store: &mut Store, delimiter: u8) -> Result<u64> {
    let f = std::fs::File::open(src)?;
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .from_reader(f);
    let headers = rdr.headers()?.clone();
    let col =
        |name: &str| -> Option<usize> { headers.iter().position(|h| h.eq_ignore_ascii_case(name)) };
    let text_col = col("text").context("CSV must have a 'text' column")?;
    let title_col = col("title");
    let uri_col = col("uri");
    let meta_col = col("meta");

    let mut n = 0u64;
    for result in rdr.records() {
        let rec = result?;
        let text = rec.get(text_col).unwrap_or("").to_string();
        if text.is_empty() {
            continue;
        }
        let title = title_col
            .and_then(|i| rec.get(i))
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let uri = uri_col
            .and_then(|i| rec.get(i))
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let meta = meta_col
            .and_then(|i| rec.get(i))
            .and_then(|s| serde_json::from_str(s).ok());
        store.put(&PutRequest {
            title,
            uri,
            text,
            meta,
            embedding: None,
        })?;
        n += 1;
    }
    Ok(n)
}

fn import_jsonl(src: &Path, store: &mut Store) -> Result<u64> {
    let f = std::fs::File::open(src)?;
    let reader = std::io::BufReader::new(f);
    let mut n = 0u64;
    for (line_no, line) in reader.lines().enumerate() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let v: serde_json::Value =
            serde_json::from_str(trimmed).with_context(|| format!("jsonl line {}", line_no + 1))?;
        let req = json_to_put(&v)?;
        store.put(&req)?;
        n += 1;
    }
    Ok(n)
}

fn import_json(src: &Path, store: &mut Store) -> Result<u64> {
    let s = std::fs::read_to_string(src)?;
    let v: serde_json::Value = serde_json::from_str(&s)?;
    let items = match &v {
        serde_json::Value::Array(arr) => arr.iter().collect::<Vec<_>>(),
        obj @ serde_json::Value::Object(_) => vec![obj],
        _ => bail!("JSON must be an array or object"),
    };
    let mut n = 0u64;
    for item in items {
        store.put(&json_to_put(item)?)?;
        n += 1;
    }
    Ok(n)
}

fn json_to_put(v: &serde_json::Value) -> Result<PutRequest> {
    let obj = v.as_object().context("expected JSON object")?;
    let text = obj
        .get("text")
        .and_then(|t| t.as_str())
        .context("missing 'text' field")?
        .to_string();
    let title = obj.get("title").and_then(|t| t.as_str()).map(str::to_owned);
    let uri = obj.get("uri").and_then(|t| t.as_str()).map(str::to_owned);
    let meta = obj.get("meta").cloned();
    Ok(PutRequest {
        title,
        uri,
        text,
        meta,
        embedding: None,
    })
}

#[cfg(feature = "import-parquet")]
fn import_parquet(src: &Path, store: &mut Store) -> Result<u64> {
    use arrow_array::RecordBatch;
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    let f = std::fs::File::open(src)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(f)?;
    let schema = builder.schema().clone();
    let col_idx = |name: &str| -> Option<usize> {
        schema
            .fields()
            .iter()
            .position(|f| f.name().eq_ignore_ascii_case(name))
    };
    let text_idx = col_idx("text").context("Parquet must have a 'text' column")?;
    let title_idx = col_idx("title");
    let uri_idx = col_idx("uri");
    let meta_idx = col_idx("meta");

    let reader = builder.build()?;
    let mut n = 0u64;
    for batch_result in reader {
        let batch: RecordBatch = batch_result?;
        let text_col = batch
            .column(text_idx)
            .as_any()
            .downcast_ref::<arrow_array::StringArray>()
            .context("'text' column must be String type")?;

        for row in 0..batch.num_rows() {
            if text_col.is_null(row) {
                continue;
            }
            let text = text_col.value(row).to_string();
            let title = title_idx.and_then(|i| {
                batch
                    .column(i)
                    .as_any()
                    .downcast_ref::<arrow_array::StringArray>()
                    .and_then(|a| {
                        if a.is_null(row) {
                            None
                        } else {
                            Some(a.value(row).to_string())
                        }
                    })
            });
            let uri = uri_idx.and_then(|i| {
                batch
                    .column(i)
                    .as_any()
                    .downcast_ref::<arrow_array::StringArray>()
                    .and_then(|a| {
                        if a.is_null(row) {
                            None
                        } else {
                            Some(a.value(row).to_string())
                        }
                    })
            });
            let meta = meta_idx.and_then(|i| {
                batch
                    .column(i)
                    .as_any()
                    .downcast_ref::<arrow_array::StringArray>()
                    .and_then(|a| {
                        if a.is_null(row) {
                            None
                        } else {
                            serde_json::from_str(a.value(row)).ok()
                        }
                    })
            });
            store.put(&PutRequest {
                title,
                uri,
                text,
                meta,
                embedding: None,
            })?;
            n += 1;
        }
    }
    Ok(n)
}

// ── Export ────────────────────────────────────────────────────────────────────

/// Export from `store` to `dst`.
pub fn export(store: &Store, dst: &Path, forced: Option<&str>) -> Result<u64> {
    match detect(dst, forced)? {
        Format::Synx => {
            let db_path = std::path::PathBuf::from(store.conn.path().context("store has no path")?);
            snap::export(&db_path, dst, 3)?;
            let count: i64 = store
                .conn
                .query_row("SELECT COUNT(*) FROM docs", [], |r| r.get(0))?;
            Ok(count as u64)
        }
        Format::Csv => export_dsv(store, dst, false),
        Format::Tsv => export_dsv(store, dst, true),
        Format::Jsonl | Format::Json => export_jsonl(store, dst),
        #[cfg(feature = "import-parquet")]
        Format::Parquet => export_parquet(store, dst),
        Format::Sqlite => {
            let mut dst_conn = rusqlite::Connection::open(dst)?;
            let backup = rusqlite::backup::Backup::new(&store.conn, &mut dst_conn)?;
            backup.run_to_completion(100, std::time::Duration::from_millis(0), None)?;
            let count: i64 = store
                .conn
                .query_row("SELECT COUNT(*) FROM docs", [], |r| r.get(0))?;
            Ok(count as u64)
        }
        Format::Lance => {
            bail!("Lance export not supported — export to .synx then use synapse-migrate")
        }
        Format::Qdrant => {
            bail!("Qdrant export not supported — export to .synx then use synapse-migrate")
        }
    }
}

fn all_docs(store: &Store) -> Result<Vec<ExportDocRow>> {
    let mut stmt = store
        .conn
        .prepare("SELECT id, title, uri, text, meta, ts FROM docs ORDER BY id")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

fn export_dsv(store: &Store, dst: &Path, tsv: bool) -> Result<u64> {
    let docs = all_docs(store)?;
    let f = std::fs::File::create(dst)?;
    let delim = if tsv { b'\t' } else { b',' };
    let mut wtr = csv::WriterBuilder::new().delimiter(delim).from_writer(f);
    wtr.write_record(["id", "title", "uri", "text", "ts"])?;
    for (id, title, uri, text, _meta, ts) in &docs {
        wtr.write_record(&[
            id.to_string(),
            title.clone().unwrap_or_default(),
            uri.clone().unwrap_or_default(),
            text.clone(),
            ts.to_string(),
        ])?;
    }
    wtr.flush()?;
    Ok(docs.len() as u64)
}

fn export_jsonl(store: &Store, dst: &Path) -> Result<u64> {
    let docs = all_docs(store)?;
    let mut f = std::io::BufWriter::new(std::fs::File::create(dst)?);
    for (id, title, uri, text, meta_str, ts) in &docs {
        let meta: Option<serde_json::Value> = meta_str
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok());
        let obj = serde_json::json!({
            "id": id,
            "title": title,
            "uri": uri,
            "text": text,
            "meta": meta,
            "ts": ts,
        });
        writeln!(f, "{}", serde_json::to_string(&obj)?)?;
    }
    f.flush()?;
    Ok(docs.len() as u64)
}

#[cfg(feature = "import-parquet")]
fn export_parquet(store: &Store, dst: &Path) -> Result<u64> {
    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use arrow_schema::{DataType, Field, Schema};
    use parquet::arrow::ArrowWriter;
    use std::sync::Arc;

    let docs = all_docs(store)?;
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("title", DataType::Utf8, true),
        Field::new("uri", DataType::Utf8, true),
        Field::new("text", DataType::Utf8, false),
        Field::new("meta", DataType::Utf8, true),
        Field::new("ts", DataType::Int64, false),
    ]));

    let ids: Vec<i64> = docs.iter().map(|(id, ..)| *id).collect();
    let titles: Vec<Option<&str>> = docs.iter().map(|(_, t, ..)| t.as_deref()).collect();
    let uris: Vec<Option<&str>> = docs.iter().map(|(_, _, u, ..)| u.as_deref()).collect();
    let texts: Vec<&str> = docs.iter().map(|(_, _, _, t, ..)| t.as_str()).collect();
    let metas: Vec<Option<&str>> = docs.iter().map(|(_, _, _, _, m, _)| m.as_deref()).collect();
    let tss: Vec<i64> = docs.iter().map(|(_, _, _, _, _, ts)| *ts).collect();

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(StringArray::from(titles)),
            Arc::new(StringArray::from(uris)),
            Arc::new(StringArray::from(texts)),
            Arc::new(StringArray::from(metas)),
            Arc::new(Int64Array::from(tss)),
        ],
    )?;

    let f = std::fs::File::create(dst)?;
    let mut writer = ArrowWriter::try_new(f, schema, None)?;
    writer.write(&batch)?;
    writer.close()?;
    Ok(docs.len() as u64)
}
