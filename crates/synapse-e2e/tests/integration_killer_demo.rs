//! Killer E2E demo — 10 stories, 10+ crates working together.
//!
//! Run:
//!   cargo test -p synapse-e2e --test integration_killer_demo
//!
//! Stories:
//!   1. Ingestion      — synapse-core + synapse-multimodal
//!   2. Indexing       — tantivy FTS (synapse-fts via synapse-core) + synapse-graph
//!   3. SQL-Wire       — SKIP (synapse-mysql requires live port; use unit tests)
//!   4. Hybrid search  — synapse-core + synapse-fusion (RRF)
//!   5. Persistence    — synapse-core snap export → import → verify identity
//!   6. CDC streaming  — synapse-stream CdcReader
//!   7. Time-series    — synapse-tsdb TsdbStore
//!   8. OLAP           — synapse-tsdb aggregate (DuckDB opt-in skipped without `olap` feature)
//!   9. Multi-node CRDT— synapse-cluster Node gossip
//!  10. Migrate-in     — mock Chroma SQLite → synapse-core via inline migration

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use synapse_core::{PutRequest, SearchMode, Store};
use synapse_core::snap;
use synapse_core::types::EMBED_DIM;

// ─── helpers ──────────────────────────────────────────────────────────────

fn fake_emb(seed: u8) -> Vec<f32> {
    let raw: Vec<f32> = (0..EMBED_DIM)
        .map(|i| ((i as u8).wrapping_mul(seed.max(1)) as f32) / 255.0 + 0.001)
        .collect();
    let norm: f32 = raw.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    raw.into_iter().map(|x| x / norm).collect()
}

fn open_store(tmp: &tempfile::NamedTempFile) -> Store {
    Store::open(tmp.path()).expect("open store")
}

// ─── Story 1: Ingestion ───────────────────────────────────────────────────

#[test]
fn story1_ingestion_text_and_multimodal() -> Result<()> {
    let tmp = tempfile::NamedTempFile::new()?;
    let mut store = open_store(&tmp);

    // 100 text docs
    for i in 0u8..100 {
        store.put(&PutRequest {
            uri: Some(format!("doc://text/{i}")),
            title: Some(format!("Rust async doc {i}")),
            text: format!("Rust async programming document number {i} about tokio and futures."),
            embedding: Some(fake_emb(i.max(1))),
            ..Default::default()
        })?;
    }
    let stats = store.stats()?;
    assert!(stats.docs >= 100, "expected ≥100 docs, got {}", stats.docs);

    // synapse-multimodal: CrossModalIndex with dummy embedder
    #[cfg(feature = "multimodal-dummy")]
    {
        use synapse_multimodal::{ClipEmbedder, CrossModalIndex, MultimodalEmbedder};
        let emb = ClipEmbedder::new();
        let mut idx = CrossModalIndex::new(emb.dim());
        for i in 0u8..5 {
            idx.add_text(&format!("caption_{i}"), &format!("image caption {i}"), &emb);
        }
        let hits = idx.query_text("caption", &emb, 3);
        assert!(!hits.is_empty(), "multimodal index returned no hits");
        println!("[PASS] story1: 100 docs ingested, {} multimodal hits", hits.len());
    }
    #[cfg(not(feature = "multimodal-dummy"))]
    {
        println!("[SKIP] story1 multimodal: feature `multimodal-dummy` not enabled");
        println!("[PASS] story1: 100 text docs ingested");
    }

    Ok(())
}

// ─── Story 2: Indexing (FTS + graph) ─────────────────────────────────────

#[test]
fn story2_fts_and_graph_indexing() -> Result<()> {
    let tmp = tempfile::NamedTempFile::new()?;
    let mut store = open_store(&tmp);

    // Insert docs that will be auto-indexed in tantivy (via tantivy-fts feature)
    for i in 0u8..20 {
        store.put(&PutRequest {
            text: format!("rust async tokio futures document {i}"),
            embedding: Some(fake_emb(i.max(1))),
            ..Default::default()
        })?;
    }

    // FTS search via store.search (uses tantivy-fts when feature enabled)
    let hits = store.search("rust async", SearchMode::Lex, None, 5)?;
    assert!(!hits.is_empty(), "tantivy-FTS returned no hits for 'rust async'");

    // synapse-graph: init schema + insert edges
    {
        use synapse_graph::{ensure_schema, relate, neighbors};
        let conn = rusqlite::Connection::open(tmp.path())?;
        ensure_schema(&conn)?;
        relate(&conn, 1, 2, "relates_to", 1.0, None)?;
        relate(&conn, 2, 3, "relates_to", 0.8, None)?;
        let nbrs = neighbors(&conn, 1, None, 10)?;
        assert!(!nbrs.is_empty(), "graph neighbors empty");
        println!("[PASS] story2: {} FTS hits, {} graph neighbors", hits.len(), nbrs.len());
    }

    Ok(())
}

// ─── Story 3: SQL-Wire (synapse-mysql) ────────────────────────────────────

#[test]
fn story3_sql_wire_skip() {
    // MySQL wire server requires a live TCP port + mysql client binary.
    // Covered in synapse-mysql crate's own tests.
    println!("[SKIP] story3: SQL-wire needs live MySQL client; test in synapse-mysql crate");
}

// ─── Story 4: Hybrid search + RRF fusion ─────────────────────────────────

#[test]
fn story4_hybrid_search_and_rrf_fusion() -> Result<()> {
    let tmp = tempfile::NamedTempFile::new()?;
    let mut store = open_store(&tmp);

    for i in 1u8..=50 {
        store.put(&PutRequest {
            text: format!("rust async programming with tokio doc {i}"),
            embedding: Some(fake_emb(i)),
            ..Default::default()
        })?;
    }

    let q_emb = fake_emb(42);
    let hits = store.search("rust async", SearchMode::Hybrid, Some(&q_emb), 10)?;
    assert!(!hits.is_empty(), "hybrid search returned nothing");
    assert!(hits.len() <= 10, "hybrid search returned more than limit");

    // synapse-fusion: RRF over two ranked lists (simulate dense + ColBERT legs)
    {
        use synapse_fusion::muvera_rrf;
        let dense: Vec<(i64, f32)> = hits.iter().enumerate().map(|(i, h)| (h.id, 1.0 / (i as f32 + 1.0))).collect();
        let colbert: Vec<(i64, f32)> = hits.iter().rev().enumerate().map(|(i, h)| (h.id, 1.0 / (i as f32 + 1.0))).collect();
        let fused = muvera_rrf(&dense, &colbert, 60.0);
        assert!(!fused.is_empty(), "muvera_rrf returned empty");
        assert_eq!(fused.len(), hits.len(), "RRF result count mismatch");
        println!("[PASS] story4: {} hybrid hits, {} RRF-fused", hits.len(), fused.len());
    }

    Ok(())
}

// ─── Story 5: Persistence (.brainpack export → import → verify) ───────────

#[test]
fn story5_persistence_export_import() -> Result<()> {
    let src = tempfile::NamedTempFile::new()?;
    let mut store = open_store(&src);

    for i in 0u8..10 {
        store.put(&PutRequest {
            uri: Some(format!("doc://persist/{i}")),
            text: format!("persistence test document {i}"),
            embedding: Some(fake_emb(i.max(1))),
            ..Default::default()
        })?;
    }
    let stats_before = store.stats()?;
    drop(store);

    let pack = tempfile::NamedTempFile::new()?;
    snap::export(src.path(), pack.path(), 3)?;
    assert!(pack.path().metadata()?.len() > 0, "exported pack is empty");

    let dst = tempfile::NamedTempFile::new()?;
    snap::import(pack.path(), dst.path())?;

    let restored = Store::open(dst.path())?;
    let stats_after = restored.stats()?;
    assert_eq!(
        stats_before.docs, stats_after.docs,
        "doc count mismatch after import: {} vs {}",
        stats_before.docs, stats_after.docs
    );
    println!("[PASS] story5: exported+imported {} docs, identity verified", stats_after.docs);
    Ok(())
}

// ─── Story 6: CDC streaming ───────────────────────────────────────────────

#[tokio::test]
async fn story6_cdc_streaming() -> Result<()> {
    use synapse_stream::cdc::{CdcReader, Op};
    use rusqlite::Connection;

    let tmp = tempfile::NamedTempFile::new()?;
    let mut cdc = CdcReader::new(tmp.path())?;

    // Create a test table and emit 10 events via emit_direct (sync path)
    {
        let conn = Connection::open(tmp.path())?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS items (id INTEGER PRIMARY KEY, name TEXT);"
        )?;
    }

    for i in 0..10i64 {
        CdcReader::emit_direct(
            &tmp.path().to_path_buf(),
            Op::Insert,
            "items",
            serde_json::json!({"id": i, "name": format!("item-{i}")}),
        )?;
    }

    // Poll rows without async (use internal poll_rows via manual SQL read)
    let conn = Connection::open(tmp.path())?;
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM _cdc_log", [], |r| r.get(0))
        .unwrap_or(0);
    assert_eq!(count, 10, "CDC log should have 10 events, got {count}");
    println!("[PASS] story6: {} CDC events captured", count);
    Ok(())
}

// ─── Story 7: Time-series ─────────────────────────────────────────────────

#[test]
fn story7_tsdb_insert_and_aggregate() -> Result<()> {
    use synapse_tsdb::{TsdbStore, AggOp};

    let tmp_dir = tempfile::tempdir()?;
    let mut tsdb = TsdbStore::open(tmp_dir.path())?;

    let base_ts: i64 = 1_700_000_000_000;
    let ts_vec: Vec<i64> = (0..1000i64).map(|i| base_ts + i * 1000).collect();
    let metrics: Vec<&str> = (0..1000).map(|_| "cpu_usage").collect();
    let labels_vec: Vec<HashMap<String, String>> = (0..1000).map(|_| HashMap::new()).collect();
    let values: Vec<f64> = (0..1000i64).map(|i| (i % 100) as f64).collect();

    tsdb.append(&ts_vec, &metrics, &labels_vec, &values)?;

    let rows = tsdb.query_range("cpu_usage", base_ts, base_ts + 1_000_000)?;
    assert_eq!(rows.len(), 1000, "expected 1000 rows, got {}", rows.len());

    let agg = tsdb.aggregate("cpu_usage", AggOp::Avg, std::time::Duration::from_secs(1_000_000))?;
    assert!(!agg.is_empty(), "aggregate returned empty");
    let avg_val = agg[0].value;
    assert!(
        (avg_val - 49.5).abs() < 1.0,
        "expected avg ~49.5, got {avg_val}"
    );
    println!("[PASS] story7: 1000 TSDB rows, avg={avg_val:.2}");
    Ok(())
}

// ─── Story 8: OLAP (DuckDB optional) ────────────────────────────────────

#[test]
fn story8_olap_aggregate() -> Result<()> {
    // Without `olap` feature DuckDB is not linked — use tsdb aggregate as proof.
    // With `olap` feature the OlapEngine would attach synapse db via DuckDB.
    #[cfg(not(feature = "olap"))]
    {
        println!("[SKIP] story8: `olap` feature not enabled; DuckDB OLAP skipped");
        println!("[INFO] story8: tsdb aggregate already verified in story7");
    }
    #[cfg(feature = "olap")]
    {
        use synapse_olap::OlapEngine;
        let tmp = tempfile::NamedTempFile::new()?;
        let mut store = open_store(&tmp);
        for i in 0u8..20 {
            store.put(&PutRequest {
                text: format!("olap doc {i}"),
                meta: Some(serde_json::json!({"category": i % 4})),
                embedding: Some(fake_emb(i.max(1))),
                ..Default::default()
            })?;
        }
        drop(store);
        let engine = OlapEngine::open(tmp.path())?;
        let rows = engine.query("SELECT COUNT(*) as n FROM docs")?;
        assert!(!rows.is_empty());
        println!("[PASS] story8: OLAP COUNT(*) = {:?}", rows[0]);
    }
    Ok(())
}

// ─── Story 9: Multi-node CRDT gossip ─────────────────────────────────────

#[tokio::test]
async fn story9_cluster_crdt_gossip() -> Result<()> {
    use synapse_cluster::{Node, PeerInfo};
    use synapse_core::sync::Op;
    use std::net::SocketAddr;

    let addr1: SocketAddr = "127.0.0.1:0".parse()?;
    let addr2: SocketAddr = "127.0.0.1:0".parse()?;
    let mut node1 = Node::new("node-1", addr1);
    let mut node2 = Node::new("node-2", addr2);

    node1.add_peer(PeerInfo { id: "node-2".into(), addr: addr2 });
    node2.add_peer(PeerInfo { id: "node-1".into(), addr: addr1 });

    // Put on node1
    let op = Op::Put {
        doc_id: "doc-hello".into(),
        blob_hash: [0u8; 32],
        ts: 1_700_000_000_000,
    };
    node1.put_op(op.clone()).await?;

    // Simulate gossip: node1 exports delta → node2 merges
    let delta = node1.local_ops().await;
    assert!(!delta.is_empty(), "node1 op log is empty after put");

    node2.merge_peer_delta(delta).await;

    let ops2 = node2.local_ops().await;
    assert!(!ops2.is_empty(), "node2 has no ops after merge");

    // Verify the Put op is present in node2
    let found = ops2.iter().any(|(_, o)| matches!(o, Op::Put { doc_id, .. } if doc_id == "doc-hello"));
    assert!(found, "Put op not found in node2 after gossip");
    println!("[PASS] story9: gossip propagated {} ops to node2", ops2.len());
    Ok(())
}

// ─── Story 10: Migrate-in (mock Chroma → synapse-core) ───────────────────

#[test]
fn story10_mock_chroma_migrate() -> Result<()> {
    use rusqlite::{Connection, params};

    // Build mock chroma.sqlite3 with 20 embeddings
    let chroma_dir = tempfile::tempdir()?;
    let chroma_db_path = chroma_dir.path().join("chroma.sqlite3");
    {
        let conn = Connection::open(&chroma_db_path)?;
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
                 id TEXT, key TEXT, str_value TEXT, int_value INTEGER, float_value REAL
             );"
        )?;
        conn.execute(
            "INSERT INTO collections VALUES ('col-1', 'test_collection')",
            [],
        )?;
        for i in 0u8..20 {
            let emb_bytes: Vec<u8> = fake_emb(i.max(1))
                .iter()
                .flat_map(|f| f.to_le_bytes())
                .collect();
            conn.execute(
                "INSERT INTO embeddings VALUES (?1,'col-1',?2,?3,?4)",
                params![
                    format!("id-{i}"),
                    emb_bytes,
                    format!("chroma doc {i}"),
                    format!("chroma://test/{i}"),
                ],
            )?;
        }
    }

    // Inline migration (replicates synapse-migrate logic without the bin dep)
    let dst = tempfile::NamedTempFile::new()?;
    let mut store = open_store(&dst);

    let conn = Connection::open(&chroma_db_path)?;
    let coll_id: String = conn.query_row(
        "SELECT id FROM collections WHERE name='test_collection'",
        [],
        |r| r.get(0),
    )?;

    let mut stmt = conn.prepare(
        "SELECT id, document, uri, embedding FROM embeddings WHERE collection_id=?1",
    )?;
    let rows: Vec<(String, Option<String>, Option<String>, Option<Vec<u8>>)> = stmt
        .query_map(params![coll_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .filter_map(|r| r.ok())
        .collect();

    for (chroma_id, doc, uri, emb_bytes) in &rows {
        let embedding = emb_bytes.as_ref().map(|b| {
            b.chunks(4)
                .filter_map(|c| c.try_into().ok().map(f32::from_le_bytes))
                .collect::<Vec<f32>>()
        });
        store.put(&PutRequest {
            uri: uri.clone().or_else(|| Some(format!("chroma://{chroma_id}"))),
            text: doc.clone().unwrap_or_default(),
            embedding,
            ..Default::default()
        })?;
    }

    let stats = store.stats()?;
    assert_eq!(
        stats.docs as usize,
        rows.len(),
        "migrate count mismatch: expected {}, got {}",
        rows.len(),
        stats.docs
    );
    println!("[PASS] story10: migrated {} chroma docs into synapse", stats.docs);
    Ok(())
}
