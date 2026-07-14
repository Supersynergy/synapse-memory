/// Smoke tests for import/export logic.
/// Re-implements the core logic inline since synapse-cli is a [[bin]] crate.
use rusqlite::Connection;
use std::io::Write;
use synapse_core::{PutRequest, Store};
use tempfile::tempdir;

fn seed_store(dir: &tempfile::TempDir) -> Store {
    let db = dir.path().join("brain.db");
    let mut store = Store::open(&db).unwrap();
    store
        .put(&PutRequest {
            text: "hello world".into(),
            title: Some("t1".into()),
            ..Default::default()
        })
        .unwrap();
    store
        .put(&PutRequest {
            text: "second doc".into(),
            uri: Some("uri:2".into()),
            ..Default::default()
        })
        .unwrap();
    store
}

fn doc_count(store: &Store) -> i64 {
    store
        .conn
        .query_row("SELECT COUNT(*) FROM docs", [], |r| r.get(0))
        .unwrap()
}

// ── CSV round-trip ────────────────────────────────────────────────────────────

#[test]
fn csv_export_then_import() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();
    let store = seed_store(&src_dir);

    // Export to CSV
    let csv_path = src_dir.path().join("out.csv");
    {
        let mut wtr = csv::WriterBuilder::new()
            .delimiter(b',')
            .from_path(&csv_path)
            .unwrap();
        wtr.write_record(["id", "title", "uri", "text", "ts"])
            .unwrap();
        let mut stmt = store
            .conn
            .prepare("SELECT id, title, uri, text, ts FROM docs ORDER BY id")
            .unwrap();
        let rows: Vec<(i64, Option<String>, Option<String>, String, i64)> = stmt
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        for (id, title, uri, text, ts) in &rows {
            wtr.write_record(&[
                id.to_string(),
                title.clone().unwrap_or_default(),
                uri.clone().unwrap_or_default(),
                text.clone(),
                ts.to_string(),
            ])
            .unwrap();
        }
        wtr.flush().unwrap();
    }

    // Import from CSV
    let db2 = dst_dir.path().join("brain2.db");
    let mut store2 = Store::open(&db2).unwrap();
    let f = std::fs::File::open(&csv_path).unwrap();
    let mut rdr = csv::ReaderBuilder::new().delimiter(b',').from_reader(f);
    let headers = rdr.headers().unwrap().clone();
    let text_col = headers
        .iter()
        .position(|h| h.eq_ignore_ascii_case("text"))
        .unwrap();
    let title_col = headers.iter().position(|h| h.eq_ignore_ascii_case("title"));
    let uri_col = headers.iter().position(|h| h.eq_ignore_ascii_case("uri"));
    for rec in rdr.records() {
        let rec = rec.unwrap();
        let text = rec.get(text_col).unwrap_or("").to_string();
        if text.is_empty() {
            continue;
        }
        store2
            .put(&PutRequest {
                text,
                title: title_col
                    .and_then(|i| rec.get(i))
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
                uri: uri_col
                    .and_then(|i| rec.get(i))
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
                ..Default::default()
            })
            .unwrap();
    }
    assert_eq!(doc_count(&store2), 2);
}

// ── JSONL round-trip ──────────────────────────────────────────────────────────

#[test]
fn jsonl_export_then_import() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();
    let store = seed_store(&src_dir);

    let jsonl_path = src_dir.path().join("out.jsonl");
    {
        let mut f = std::io::BufWriter::new(std::fs::File::create(&jsonl_path).unwrap());
        let mut stmt = store
            .conn
            .prepare("SELECT id, title, uri, text, ts FROM docs ORDER BY id")
            .unwrap();
        let rows: Vec<(i64, Option<String>, Option<String>, String, i64)> = stmt
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        for (id, title, uri, text, ts) in &rows {
            let obj =
                serde_json::json!({"id": id, "title": title, "uri": uri, "text": text, "ts": ts});
            writeln!(f, "{}", serde_json::to_string(&obj).unwrap()).unwrap();
        }
    }

    let db2 = dst_dir.path().join("brain2.db");
    let mut store2 = Store::open(&db2).unwrap();
    let f = std::fs::File::open(&jsonl_path).unwrap();
    use std::io::BufRead;
    for line in std::io::BufReader::new(f).lines() {
        let line = line.unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        let obj = v.as_object().unwrap();
        let text = obj["text"].as_str().unwrap().to_string();
        if text.is_empty() {
            continue;
        }
        store2
            .put(&PutRequest {
                text,
                title: obj.get("title").and_then(|t| t.as_str()).map(str::to_owned),
                uri: obj.get("uri").and_then(|t| t.as_str()).map(str::to_owned),
                ..Default::default()
            })
            .unwrap();
    }
    assert_eq!(doc_count(&store2), 2);
}

// ── SQLite export ─────────────────────────────────────────────────────────────

#[test]
fn sqlite_backup_export() {
    let src_dir = tempdir().unwrap();
    let store = seed_store(&src_dir);
    let out_db = src_dir.path().join("copy.db");
    {
        let mut dst_conn = Connection::open(&out_db).unwrap();
        let backup = rusqlite::backup::Backup::new(&store.conn, &mut dst_conn).unwrap();
        backup
            .run_to_completion(100, std::time::Duration::from_millis(0), None)
            .unwrap();
    }
    let c = Connection::open(&out_db).unwrap();
    let count: i64 = c
        .query_row("SELECT COUNT(*) FROM docs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2);
}

// ── JSON import ───────────────────────────────────────────────────────────────

#[test]
fn json_array_import() {
    let dir = tempdir().unwrap();
    let json_path = dir.path().join("data.json");
    std::fs::write(
        &json_path,
        r#"[{"text":"foo","title":"Foo"},{"text":"bar"}]"#,
    )
    .unwrap();
    let db = dir.path().join("brain.db");
    let mut store = Store::open(&db).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json_path).unwrap()).unwrap();
    for item in v.as_array().unwrap() {
        let obj = item.as_object().unwrap();
        let text = obj["text"].as_str().unwrap().to_string();
        store
            .put(&PutRequest {
                text,
                title: obj.get("title").and_then(|t| t.as_str()).map(str::to_owned),
                ..Default::default()
            })
            .unwrap();
    }
    assert_eq!(doc_count(&store), 2);
}

// ── External SQLite import ────────────────────────────────────────────────────

#[test]
fn external_sqlite_import() {
    let dir = tempdir().unwrap();
    let src_db = dir.path().join("src.db");
    {
        let conn = Connection::open(&src_db).unwrap();
        conn.execute_batch(
            "CREATE TABLE docs (id INTEGER PRIMARY KEY, title TEXT, uri TEXT, text TEXT NOT NULL)",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO docs (title, text) VALUES (?1, ?2)",
            rusqlite::params!["T1", "external doc"],
        )
        .unwrap();
    }
    let dest_db = dir.path().join("brain.db");
    let mut store = Store::open(&dest_db).unwrap();

    // Simulate import_sqlite
    let src_conn = Connection::open(&src_db).unwrap();
    let mut stmt = src_conn
        .prepare("SELECT title, NULL, text, NULL FROM docs")
        .unwrap();
    let rows: Vec<(Option<String>, Option<String>, String, Option<String>)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    for (title, uri, text, meta_str) in rows {
        let meta = meta_str.and_then(|s| serde_json::from_str(&s).ok());
        store
            .put(&PutRequest {
                title,
                uri,
                text,
                meta,
                ..Default::default()
            })
            .unwrap();
    }
    assert_eq!(doc_count(&store), 1);
    let t: String = store
        .conn
        .query_row("SELECT text FROM docs LIMIT 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(t, "external doc");
}
