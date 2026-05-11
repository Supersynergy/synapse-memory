use anyhow::{bail, Context, Result};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use synapse_core::{PutRequest, Store};

#[derive(Parser)]
#[command(name = "synapse-migrate", version, about = "Import Qdrant/LanceDB/Chroma/Pinecone/Weaviate into .synx")]
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
    } else if src.starts_with("pinecone://") {
        migrate_pinecone(src, &mut store, cli.batch, cli.offset)?
    } else if src.starts_with("weaviate://") {
        migrate_weaviate(src, &mut store, cli.batch, cli.offset)?
    } else {
        bail!("unknown source scheme — expected qdrant:// | lancedb:// | chroma:// | pinecone:// | weaviate://");
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

// ── Pinecone (HTTP REST — curl subprocess) ────────────────────────────────
//
// URI: pinecone://<index>-<project>.svc.<env>.pinecone.io/<namespace>
//   or pinecone://<host>/<namespace>  (namespace optional, defaults to "")
//
// Env: PINECONE_API_KEY
//
// Flow: POST /vectors/list → get ids → POST /vectors/fetch in batches.
// Pinecone list returns up to 100 ids per page with a pagination_token.
fn migrate_pinecone(src: &str, store: &mut Store, batch: usize, offset: u64) -> Result<u64> {
    let stripped = src.strip_prefix("pinecone://").unwrap();
    let (host, namespace) = if let Some(slash) = stripped.find('/') {
        (&stripped[..slash], &stripped[slash + 1..])
    } else {
        (stripped, "")
    };

    let api_key = std::env::var("PINECONE_API_KEY")
        .context("PINECONE_API_KEY env var required for pinecone:// source")?;

    let base_url = format!("https://{}", host);

    let pb = ProgressBar::new_spinner();
    pb.set_message("listing Pinecone vectors...");

    let mut imported: u64 = 0;
    let mut skipped: u64 = 0;
    let mut pagination_token: Option<String> = None;
    let mut id_buf: Vec<String> = Vec::new();

    // Collect all IDs first (list endpoint), then fetch in batches
    loop {
        let mut list_url = format!("{}/vectors/list?namespace={}&limit=100", base_url, namespace);
        if let Some(ref tok) = pagination_token {
            list_url.push_str(&format!("&paginationToken={}", tok));
        }

        let resp = http_get_json_with_key(&list_url, &api_key)
            .context("pinecone list request failed")?;

        let ids = resp
            .pointer("/vectors")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.get("id").and_then(|i| i.as_str()).map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        id_buf.extend(ids);

        pagination_token = resp
            .pointer("/pagination/next")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        if pagination_token.is_none() {
            break;
        }
    }

    pb.finish_with_message(format!("listed {} Pinecone vector ids", id_buf.len()));

    let total = id_buf.len() as u64;
    let pb = progress_bar(total.saturating_sub(offset));

    for chunk in id_buf.chunks(batch) {
        if skipped + chunk.len() as u64 <= offset {
            skipped += chunk.len() as u64;
            continue;
        }

        // Build fetch URL with ids as query params
        let ids_qs: String = chunk
            .iter()
            .map(|id| format!("ids={}", urlencod(id)))
            .collect::<Vec<_>>()
            .join("&");
        let fetch_url = format!("{}/vectors/fetch?namespace={}&{}", base_url, namespace, ids_qs);

        let resp = http_get_json_with_key(&fetch_url, &api_key)
            .context("pinecone fetch request failed")?;

        let vectors_obj = resp
            .pointer("/vectors")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();

        let chunk_len = vectors_obj.len() as u64;

        for (vec_id, vec_val) in &vectors_obj {
            let metadata = vec_val.get("metadata");
            let text = metadata
                .and_then(|m| m.get("text").or_else(|| m.get("content")))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let uri = metadata
                .and_then(|m| m.get("uri").or_else(|| m.get("url")))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let title = metadata
                .and_then(|m| m.get("title"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let embedding: Option<Vec<f32>> = vec_val
                .get("values")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect());

            let mut meta = metadata.cloned().unwrap_or(serde_json::Value::Object(Default::default()));
            if let Some(obj) = meta.as_object_mut() {
                obj.insert("pinecone_id".into(), serde_json::Value::String(vec_id.clone()));
                obj.insert("pinecone_namespace".into(), serde_json::Value::String(namespace.to_string()));
            }

            let req = PutRequest { uri, title, text, meta: Some(meta), embedding };
            store.put(&req)?;
        }

        imported += chunk_len;
        pb.inc(chunk_len);
    }

    pb.finish_with_message("pinecone import complete");
    Ok(imported)
}

/// Percent-encode a string (minimal — spaces and special chars only).
fn urlencod(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

// ── Weaviate (GraphQL) ────────────────────────────────────────────────────
//
// URI: weaviate://<host:port>/<ClassName>
//   default host: localhost:8080
//
// Optional: WEAVIATE_API_KEY env (for authenticated instances)
//
// Uses GraphQL { Get { <ClassName>(limit: N, offset: M) { _additional { id vector } ... } } }
fn migrate_weaviate(src: &str, store: &mut Store, batch: usize, offset: u64) -> Result<u64> {
    let stripped = src.strip_prefix("weaviate://").unwrap();
    let (host_port, class_name) = if let Some(slash) = stripped.find('/') {
        (&stripped[..slash], &stripped[slash + 1..])
    } else {
        bail!("missing class name in weaviate:// URI — expected weaviate://host:port/ClassName");
    };

    if class_name.is_empty() {
        bail!("empty class name in weaviate:// URI");
    }

    let base_url = format!("http://{}/v1/graphql", host_port);
    let api_key = std::env::var("WEAVIATE_API_KEY").ok();

    // Probe: GET /v1/meta to verify connectivity
    let meta_url = format!("http://{}/v1/meta", host_port);
    http_get_json_opt_key(&meta_url, api_key.as_deref())
        .with_context(|| format!("cannot reach Weaviate at {}", meta_url))?;

    // Count total via aggregate
    let count_query = serde_json::json!({
        "query": format!(
            "{{ Aggregate {{ {} {{ meta {{ count }} }} }} }}",
            class_name
        )
    });
    let count_resp = http_post_json_opt_key(&base_url, &count_query, api_key.as_deref())
        .context("weaviate aggregate count failed")?;
    let total_count = count_resp
        .pointer(&format!("/data/Aggregate/{}/0/meta/count", class_name))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let pb = progress_bar(total_count.saturating_sub(offset));

    let mut imported: u64 = 0;
    let mut page_offset = offset;

    loop {
        // GraphQL query: fetch id, vector, and all scalar properties
        // We request _additional { id vector } plus a generic properties block.
        // Weaviate returns unknown property names in a catch-all; we use
        // `properties { ... }` omitted so only _additional is fetched for the
        // skeleton — callers can extend via custom GraphQL.
        let gql = format!(
            r#"{{ Get {{ {class}(limit: {limit}, offset: {offset}) {{
                _additional {{ id vector }}
                text content body title uri url
            }} }} }}"#,
            class = class_name,
            limit = batch,
            offset = page_offset,
        );
        let query_body = serde_json::json!({ "query": gql });

        let resp = http_post_json_opt_key(&base_url, &query_body, api_key.as_deref())
            .context("weaviate graphql request failed")?;

        let objects = resp
            .pointer(&format!("/data/Get/{}", class_name))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        if objects.is_empty() {
            break;
        }

        let chunk_len = objects.len() as u64;

        for obj in &objects {
            let additional = obj.get("_additional");
            let weaviate_id = additional
                .and_then(|a| a.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let embedding: Option<Vec<f32>> = additional
                .and_then(|a| a.get("vector"))
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect());

            let text = obj
                .get("text")
                .or_else(|| obj.get("content"))
                .or_else(|| obj.get("body"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let title = obj
                .get("title")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let uri = obj
                .get("uri")
                .or_else(|| obj.get("url"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            // Build meta: all fields except _additional
            let mut meta_map = serde_json::Map::new();
            meta_map.insert("weaviate_id".into(), serde_json::Value::String(weaviate_id));
            meta_map.insert("weaviate_class".into(), serde_json::Value::String(class_name.to_string()));
            for (k, v) in obj.as_object().into_iter().flatten() {
                if k != "_additional" {
                    meta_map.insert(k.clone(), v.clone());
                }
            }

            let req = PutRequest {
                uri,
                title,
                text,
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

    pb.finish_with_message("weaviate import complete");
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

fn http_get_json_with_key(url: &str, api_key: &str) -> Result<serde_json::Value> {
    let output = std::process::Command::new("curl")
        .args([
            "-sf", "--max-time", "30",
            "-H", &format!("Api-Key: {}", api_key),
            url,
        ])
        .output()
        .context("curl not found")?;
    if !output.status.success() {
        bail!("curl GET {} failed: {}", url, String::from_utf8_lossy(&output.stderr));
    }
    serde_json::from_slice(&output.stdout).context("parse JSON response")
}

fn http_get_json_opt_key(url: &str, api_key: Option<&str>) -> Result<serde_json::Value> {
    let mut cmd = std::process::Command::new("curl");
    cmd.args(["-sf", "--max-time", "30"]);
    if let Some(k) = api_key {
        cmd.args(["-H", &format!("Authorization: Bearer {}", k)]);
    }
    cmd.arg(url);
    let output = cmd.output().context("curl not found")?;
    if !output.status.success() {
        bail!("curl GET {} failed: {}", url, String::from_utf8_lossy(&output.stderr));
    }
    serde_json::from_slice(&output.stdout).context("parse JSON response")
}

fn http_post_json_opt_key(url: &str, body: &serde_json::Value, api_key: Option<&str>) -> Result<serde_json::Value> {
    let body_str = body.to_string();
    let mut cmd = std::process::Command::new("curl");
    cmd.args(["-sf", "--max-time", "30", "-X", "POST", "-H", "Content-Type: application/json"]);
    if let Some(k) = api_key {
        cmd.args(["-H", &format!("Authorization: Bearer {}", k)]);
    }
    cmd.args(["-d", &body_str, url]);
    let output = cmd.output().context("curl not found")?;
    if !output.status.success() {
        bail!("curl POST {} failed: {}", url, String::from_utf8_lossy(&output.stderr));
    }
    serde_json::from_slice(&output.stdout).context("parse JSON response")
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

    /// Pinecone: no live server — verify missing API key errors cleanly.
    #[test]
    fn test_pinecone_missing_key() {
        let tmp = TempDir::new().unwrap();
        let mut store = make_store(&tmp);
        // Unset key for this test
        std::env::remove_var("PINECONE_API_KEY");
        let err = migrate_pinecone("pinecone://my-index-abc123.svc.us-east1-gcp.pinecone.io/default", &mut store, 32, 0);
        assert!(err.is_err());
        let msg = format!("{}", err.unwrap_err());
        assert!(msg.contains("PINECONE_API_KEY"), "expected key error, got: {}", msg);
    }

    /// Pinecone: verify URI without namespace errors cleanly (host-only).
    #[test]
    fn test_pinecone_no_namespace_still_ok() {
        let tmp = TempDir::new().unwrap();
        let mut store = make_store(&tmp);
        std::env::remove_var("PINECONE_API_KEY");
        // No slash → no collection, but should fail on API key first
        let err = migrate_pinecone("pinecone://myhost.svc.pinecone.io", &mut store, 32, 0);
        assert!(err.is_err());
        let msg = format!("{}", err.unwrap_err());
        assert!(msg.contains("PINECONE_API_KEY"), "got: {}", msg);
    }

    /// Weaviate: missing class name in URI errors cleanly.
    #[test]
    fn test_weaviate_bad_uri() {
        let tmp = TempDir::new().unwrap();
        let mut store = make_store(&tmp);
        let err = migrate_weaviate("weaviate://localhost:8080", &mut store, 32, 0);
        assert!(err.is_err());
        let msg = format!("{}", err.unwrap_err());
        assert!(msg.contains("class name"), "got: {}", msg);
    }

    /// Weaviate: empty class name in URI errors cleanly.
    #[test]
    fn test_weaviate_empty_class() {
        let tmp = TempDir::new().unwrap();
        let mut store = make_store(&tmp);
        let err = migrate_weaviate("weaviate://localhost:8080/", &mut store, 32, 0);
        assert!(err.is_err());
        let msg = format!("{}", err.unwrap_err());
        assert!(msg.contains("empty class"), "got: {}", msg);
    }

    /// Weaviate: unreachable host errors cleanly (curl exits non-zero).
    #[test]
    fn test_weaviate_unreachable() {
        let tmp = TempDir::new().unwrap();
        let mut store = make_store(&tmp);
        let err = migrate_weaviate("weaviate://127.0.0.1:19999/Article", &mut store, 32, 0);
        assert!(err.is_err());
    }

    #[test]
    fn test_bytes_to_f32_vec() {
        let v: Vec<u8> = 1.0f32.to_le_bytes().iter().chain(2.0f32.to_le_bytes().iter()).copied().collect();
        let result = bytes_to_f32_vec(&v).unwrap();
        assert_eq!(result, vec![1.0f32, 2.0f32]);
    }
}
