use serde_json::json;
use std::time::Duration;
use synapse_stream::cq::{ContinuousQuery, QueryEngine};
use synapse_stream::{CdcReader, Op};
use tokio_stream::StreamExt;

#[tokio::test]
async fn test_1k_inserts_cdc() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");

    let mut reader = CdcReader::new(&db).unwrap();

    // Insert 1000 events directly (no table/trigger needed for direct emit).
    for i in 0u64..1000 {
        CdcReader::emit_direct(&db, Op::Insert, "items", json!({"id": i, "v": i * 2})).unwrap();
    }

    let mut count = 0usize;
    let mut stream = reader.tail();
    while let Some(Ok(_ev)) = stream.next().await {
        count += 1;
        if count == 1000 {
            break;
        }
    }
    assert_eq!(count, 1000, "expected 1000 CDC events");
}

#[tokio::test]
async fn test_window_avg_cq() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test2.db");

    // Init reader first (creates _cdc_log table)
    let mut reader = CdcReader::new(&db).unwrap();

    // Emit 10 events with value 10..20
    for i in 10i64..20 {
        CdcReader::emit_direct(&db, Op::Insert, "metrics", json!({"val": i})).unwrap();
    }
    let mut events = vec![];
    {
        let mut stream = reader.tail();
        while let Some(Ok(ev)) = stream.next().await {
            events.push(ev);
            if events.len() == 10 {
                break;
            }
        }
    }

    let mut engine = QueryEngine::new(&db);
    engine.register_cq("avg_val", ContinuousQuery {
        sql: "SELECT AVG(CAST(json_extract(row_json,'$.val') AS REAL)) as avg_val FROM events".into(),
        window: Duration::from_secs(1),
    }).unwrap();

    let result = engine.eval("avg_val", &events).unwrap();
    assert_eq!(result.len(), 1);
    let avg = result[0]["avg_val"].as_f64().unwrap();
    assert!((avg - 14.5).abs() < 0.01, "avg_val={}", avg);
}

#[tokio::test]
async fn test_trigger_based_cdc() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test3.db");

    // Create a real table and install triggers
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT);")
            .unwrap();
    }

    let mut reader = CdcReader::new(&db).unwrap();
    reader.install_triggers("items").unwrap();

    // Insert 50 rows
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        for i in 0..50 {
            conn.execute(
                "INSERT INTO items(id,name) VALUES(?1,?2)",
                rusqlite::params![i, format!("item{}", i)],
            )
            .unwrap();
        }
    }

    let mut count = 0usize;
    let mut stream = reader.tail();
    while let Some(Ok(_ev)) = stream.next().await {
        count += 1;
        if count == 50 {
            break;
        }
    }
    assert_eq!(count, 50);
}
