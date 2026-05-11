use anyhow::{bail, Context, Result};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use synapse_core::{PutRequest, Store};

#[derive(Parser)]
#[command(name = "synapse-migrate", version, about = "Import Qdrant/LanceDB/Chroma into .synx")]
struct Cli {
    /// Source URI: qdrant://host:port/collection | lancedb:///path/table | chroma:///path/collection
    #[arg(long)]
    from: String,

    /// Destination .synx brain file (created if absent)
    #[arg(long, default_value = ".synapse/brain.db")]
    to: PathBuf,

    /// Batch size for import
    #[arg(long, default_value_t = 256)]
    batch: usize,

    /// Resume from this offset (last imported count)
    #[arg(long, default_value_t = 0)]
    offset: u64,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    if let Some(p) = cli.to.parent() {
        std::fs::create_dir_all(p).ok();
    }
    let mut store = Store::open(&cli.to).context("open destination store")?;

    let src = cli.from.as_str();

    let total = if src.starts_with("qdrant://") {
        migrate_qdrant(src, &mut store, cli.batch, cli.offset)?
    } else if src.starts_with("lancedb://") {
        migrate_lancedb(src, &mut store, cli.batch, cli.offset)?
    } else if src.starts_with("chroma://") {
        migrate_chroma(src, &mut store, cli.batch, cli.offset)?
    } else {
        bail!("unknown source scheme — expected qdrant:// | lancedb:// | chroma://");
    };

    println!("migrate done: {} docs inserted into {}", total, cli.to.display());
    Ok(())
}

// ── Chroma (fully implemented — SQLite direct) ─────────────────────────────

/// chroma:///path/to/chroma/dir/collection_name
///
/// Chroma SQLite layout (v0.4+):
///   chroma.sqlite3 tables:
///     embeddings(id TEXT, collection_id TEXT, embedding BLOB, document TEXT, uri TEXT)
///     embedding_metadata(id TEXT, key TEXT, str_value TEXT, int_value INTEGER, float_value REAL)
///     collections(id TEXT, name TEXT)
fn migrate_chroma(src: &str, store: &mut Store, batch: usize, offset: u64) -> Result<u64> {
    // Parse: chroma:///abs/path/to/chroma_dir/collection_name
    let stripped = src.strip_prefix("chroma://").unwrap();
    // Split last path component as collection name
    let (db_dir, collection_name) = {
        let p = std::path::Path::new(stripped);
        let name = p
            .file_name()
            .and_then(|n| n.to_str())
            .context("missing collection name in chroma:// URI")?
            .to_string();
        let dir = p.parent().unwrap_or(std::path::Path::new("."));
        (dir.to_path_buf(), name)
    };

    let db_path = db_dir.join("chroma.sqlite3");
    let conn = rusqlite::Connection::open(&db_path)
        .with_context(|| format!("open chroma sqlite: {}", db_path.display()))?;

    // Resolve collection id
    let coll_id: String = conn
        .query_row(
            "SELECT id FROM collections WHERE name = ?1",
            rusqlite::params![collection_name],
            |r| r.get(0),
        )
        .with_context(|| format!("collection '{}' not found in chroma db", collection_name))?;

    let total_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM embeddings WHERE collection_id = ?1",
        rusqlite::params![coll_id],
        |r| r.get(0),
    )?;

    let pb = progress_bar((total_count as u64).saturating_sub(offset));

    let mut imported: u64 = 0;
    let mut page_offset = offset;

    loop {
        let mut stmt = conn.prepare(
            "SELECT id, document, uri, embedding FROM embeddings WHERE collection_id = ?1 \
             ORDER BY rowid LIMIT ?2 OFFSET ?3",
        )?;

        let rows: Vec<(String, Option<String>, Option<String>, Option<Vec<u8>>)> = stmt
            .query_map(
                rusqlite::params![coll_id, batch as i64, page_offset as i64],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?
            .collect::<rusqlite::Result<_>>()?;

        if rows.is_empty() {
            break;
        }

        let chunk_len = rows.len() as u64;

        for (chroma_id, document, uri, emb_bytes) in rows {
            // Gather metadata for this id
            let mut meta_map = serde_json::Map::new();
            meta_map.insert("chroma_id".into(), serde_json::Value::String(chroma_id.clone()));

            let mut mstmt = conn.prepare(
                "SELECT key, str_value, int_value, float_value FROM embedding_metadata WHERE id = ?1",
            )?;
            let _ = mstmt.query_map(rusqlite::params![chroma_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<f64>>(3)?,
                ))
            })
            .and_then(|mapped| {
                for row in mapped {
                    let (key, sv, iv, fv) = row?;
                    let val = if let Some(s) = sv {
                        serde_json::Value::String(s)
                    } else if let Some(i) = iv {
                        serde_json::Value::Number(i.into())
                    } else if let Some(f) = fv {
                        serde_json::json!(f)
                    } else {
                        serde_json::Value::Null
                    };
                    meta_map.insert(key, val);
                }
                Ok(())
            });

            let embedding = emb_bytes.and_then(|b| bytes_to_f32_vec(&b));

            let req = PutRequest {
                uri,
                title: None,
                text: document.unwrap_or_default(),
                meta: Some(serde_json::Value::Object(meta_map)),
                embedding,
            };
            store.put(&req)?;
        }

        imported += chunk_len;
        pb.inc(chunk_len);
        page_offset += chunk_len;

        if chunk_len < batch as u64 {
            break;
        }
    }

    pb.finish_with_message("chroma import complete");
    Ok(imported)
}

fn bytes_to_f32_vec(b: &[u8]) -> Option<Vec<f32>> {
    if b.len() % 4 != 0 {
        return None;
    }
    Some(
        b.chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

// ── Qdrant (HTTP REST — no heavy gRPC crate) ──────────────────────────────

/// qdrant://localhost:6334/my_collection
///
/// Uses the Qdrant REST API (port 6333 by default for HTTP;
/// the URI port is passed directly — use 6333 for REST).
/// Scrolls all points with vectors and payload.
fn migrate_qdrant(src: &str, store: &mut Store, batch: usize, offset: u64) -> Result<u64> {
    let stripped = src.strip_prefix("qdrant://").unwrap();
    let slash = stripped.find('/').context("missing collection name in qdrant:// URI")?;
    let host_port = &stripped[..slash];
    let collection = &stripped[slash + 1..];

    let base_url = format!("http://{}", host_port);

    // Probe — fail fast if unreachable
    let info_url = format!("{}/collections/{}", base_url, collection);
    let info_body = http_get_json(&info_url)
        .with_context(|| format!("cannot reach Qdrant at {}", info_url))?;
    let count = info_body
        .pointer("/result/vectors_count")
        .or_else(|| info_body.pointer("/result/points_count"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let pb = progress_bar(count.saturating_sub(offset));

    let mut imported: u64 = 0;
    let mut next_offset: Option<serde_json::Value> = if offset > 0 {
        Some(serde_json::Value::Number(offset.into()))
    } else {
        None
    };

    loop {
        let mut body = serde_json::json!({
            "limit": batch,
            "with_vectors": true,
            "with_payload": true
        });
        if let Some(ref off) = next_offset {
            body["offset"] = off.clone();
        }

        let scroll_url = format!("{}/collections/{}/points/scroll", base_url, collection);
        let resp = http_post_json(&scroll_url, &body)
            .context("qdrant scroll request failed")?;

        let points = resp
            .pointer("/result/points")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        if points.is_empty() {
            break;
        }

        let chunk_len = points.len() as u64;

        for pt in &points {
            let payload = pt.get("payload");
            let content = payload
                .and_then(|p| p.get("content").or_else(|| p.get("text")))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let uri = payload
                .and_then(|p| p.get("uri").or_else(|| p.get("url")))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let title = payload
                .and_then(|p| p.get("title"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let embedding: Option<Vec<f32>> = pt
                .get("vector")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect());

            let mut meta = payload.cloned().unwrap_or(serde_json::Value::Object(Default::default()));
            if let Some(id) = pt.get("id") {
                if let Some(obj) = meta.as_object_mut() {
                    obj.insert("qdrant_id".into(), id.clone());
                }
            }

            let req = PutRequest {
                uri,
                title,
                text: content,
                meta: Some(meta),
                embedding,
            };
            store.put(&req)?;
        }

        imported += chunk_len;
        pb.inc(chunk_len);

        next_offset = resp.pointer("/result/next_page_offset").cloned();
        if next_offset.is_none() || next_offset == Some(serde_json::Value::Null) {
            break;
        }
    }

    pb.finish_with_message("qdrant import complete");
    Ok(imported)
}

// ── LanceDB (skeleton — subprocess via lance CLI or direct table file) ─────

/// lancedb:///path/to/db/table_name
///
/// Skeleton: reads Lance Arrow IPC files directly from the table directory.
/// Full Arrow IPC parsing requires the `arrow` crate (heavy); this skeleton
/// does a row-count scan and emits one doc per row with metadata only.
/// Replace `parse_lance_row` with proper Arrow decoding for production.
fn migrate_lancedb(src: &str, store: &mut Store, _batch: usize, offset: u64) -> Result<u64> {
    let stripped = src.strip_prefix("lancedb://").unwrap();
    let (db_path_str, table_name) = {
        let p = std::path::Path::new(stripped);
        let name = p
            .file_name()
            .and_then(|n| n.to_str())
            .context("missing table name in lancedb:// URI")?
            .to_string();
        let dir = p.parent().unwrap_or(std::path::Path::new("."));
        (dir.to_string_lossy().to_string(), name)
    };

    // LanceDB stores data as <db>/<table>.lance/data/*.lance files
    let table_dir = std::path::Path::new(&db_path_str).join(format!("{}.lance", table_name));
    let data_dir = table_dir.join("data");

    if !data_dir.exists() {
        bail!(
            "LanceDB table directory not found: {} — enable feature migrate-lancedb for full Arrow support",
            data_dir.display()
        );
    }

    // Count .lance fragment files
    let fragments: Vec<_> = std::fs::read_dir(&data_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |x| x == "lance"))
        .collect();

    if fragments.is_empty() {
        bail!("no .lance fragment files found in {}", data_dir.display());
    }

    eprintln!(
        "[lancedb] SKELETON mode: found {} fragment(s) in {}",
        fragments.len(),
        data_dir.display()
    );
    eprintln!(
        "[lancedb] Full Arrow IPC decoding requires the `arrow` crate (feature migrate-lancedb)."
    );
    eprintln!("[lancedb] Inserting placeholder docs for each fragment.");

    let pb = progress_bar(fragments.len() as u64);
    let mut imported: u64 = 0;

    for (i, frag) in fragments.iter().enumerate() {
        if (i as u64) < offset {
            continue;
        }
        let frag_name = frag.file_name().to_string_lossy().to_string();
        let req = PutRequest {
            uri: Some(format!("lancedb://{}/{}", table_name, frag_name)),
            title: Some(format!("LanceDB fragment {}", frag_name)),
            text: format!(
                "LanceDB table '{}' fragment '{}' — re-run with full Arrow support to decode content.",
                table_name, frag_name
            ),
            meta: Some(serde_json::json!({
                "source": "lancedb",
                "table": table_name,
                "fragment": frag_name,
                "skeleton": true
            })),
            embedding: None,
        };
        store.put(&req)?;
        imported += 1;
        pb.inc(1);
    }

    pb.finish_with_message("lancedb skeleton import complete");
    Ok(imported)
}

// ── HTTP helpers (no reqwest — keep deps minimal) ─────────────────────────

fn http_get_json(url: &str) -> Result<serde_json::Value> {
    let output = std::process::Command::new("curl")
        .args(["-sf", "--max-time", "30", url])
        .output()
        .context("curl not found")?;
    if !output.status.success() {
        bail!("curl GET {} failed: {}", url, String::from_utf8_lossy(&output.stderr));
    }
    let v: serde_json::Value = serde_json::from_slice(&output.stdout)
        .context("parse JSON response")?;
    Ok(v)
}

fn http_post_json(url: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
    let body_str = body.to_string();
    let output = std::process::Command::new("curl")
        .args([
            "-sf",
            "--max-time",
            "30",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "-d",
            &body_str,
            url,
        ])
        .output()
        .context("curl not found")?;
    if !output.status.success() {
        bail!("curl POST {} failed: {}", url, String::from_utf8_lossy(&output.stderr));
    }
    let v: serde_json::Value = serde_json::from_slice(&output.stdout)
        .context("parse JSON response")?;
    Ok(v)
}

fn progress_bar(total: u64) -> ProgressBar {
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})",
        )
        .unwrap()
        .progress_chars("=> "),
    );
    pb
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_store(tmp: &TempDir) -> Store {
        Store::open(tmp.path().join("brain.db")).unwrap()
    }

    /// Build a minimal Chroma SQLite in a temp dir, migrate 100 docs, verify count.
    #[test]
    fn test_chroma_smoke() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("chroma.sqlite3");

        // Build fake chroma DB
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE collections (id TEXT PRIMARY KEY, name TEXT);
             CREATE TABLE embeddings (
               id TEXT PRIMARY KEY,
               collection_id TEXT,
               embedding BLOB,
               document TEXT,
               uri TEXT
             );
             CREATE TABLE embedding_metadata (
               id TEXT, key TEXT, str_value TEXT,
               int_value INTEGER, float_value REAL
             );",
        )
        .unwrap();

        conn.execute(
            "INSERT INTO collections VALUES ('coll-1', 'test')",
            [],
        )
        .unwrap();

        for i in 0u32..100 {
            let fake_emb: Vec<u8> = (0u32..384).flat_map(|_| (i as f32).to_le_bytes()).collect();
            conn.execute(
                "INSERT INTO embeddings VALUES (?1,'coll-1',?2,?3,NULL)",
                rusqlite::params![
                    format!("id-{}", i),
                    fake_emb,
                    format!("document number {}", i)
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO embedding_metadata VALUES (?1,'idx',NULL,?2,NULL)",
                rusqlite::params![format!("id-{}", i), i as i64],
            )
            .unwrap();
        }
        drop(conn);

        let src = format!("chroma://{}/test", tmp.path().display());
        let mut store = make_store(&tmp);
        let count = migrate_chroma(&src, &mut store, 32, 0).unwrap();
        assert_eq!(count, 100, "expected 100 docs migrated");

        // Verify stored in synapse
        let hits = store.search("document", synapse_core::SearchMode::Lex, None, 200).unwrap();
        assert!(hits.len() >= 100, "expected >= 100 hits, got {}", hits.len());
    }

    /// Qdrant source: no live server — just verify URI parse error for bad scheme.
    #[test]
    fn test_qdrant_bad_uri() {
        let tmp = TempDir::new().unwrap();
        let mut store = make_store(&tmp);
        let err = migrate_qdrant("qdrant://localhost:6333", &mut store, 32, 0);
        assert!(err.is_err(), "expected error for missing collection");
    }

    /// LanceDB skeleton: no real Lance file — verify it errors cleanly.
    #[test]
    fn test_lancedb_missing() {
        let tmp = TempDir::new().unwrap();
        let mut store = make_store(&tmp);
        let src = format!("lancedb://{}/nonexistent", tmp.path().display());
        let err = migrate_lancedb(&src, &mut store, 32, 0);
        assert!(err.is_err());
    }

    /// LanceDB skeleton: create fake fragment dir, verify placeholder import.
    #[test]
    fn test_lancedb_skeleton_smoke() {
        let tmp = TempDir::new().unwrap();
        let frag_dir = tmp.path().join("mytable.lance").join("data");
        std::fs::create_dir_all(&frag_dir).unwrap();
        // Create 5 fake fragment files
        for i in 0..5 {
            std::fs::write(frag_dir.join(format!("{:020}.lance", i)), b"fake").unwrap();
        }

        let src = format!("lancedb://{}/mytable", tmp.path().display());
        let mut store = make_store(&tmp);
        let count = migrate_lancedb(&src, &mut store, 32, 0).unwrap();
        assert_eq!(count, 5);
    }

    #[test]
    fn test_bytes_to_f32_vec() {
        let v: Vec<u8> = 1.0f32.to_le_bytes().iter().chain(2.0f32.to_le_bytes().iter()).copied().collect();
        let result = bytes_to_f32_vec(&v).unwrap();
        assert_eq!(result, vec![1.0f32, 2.0f32]);
    }
}
