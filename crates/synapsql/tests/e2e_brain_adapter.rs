//! E2E smoke tests for BrainAdapter — synapse-core::Store wired to LibsqlStore.
//!
//! No MySQL client needed: we call BrainAdapter directly via the LibsqlStore trait.

use std::sync::Arc;
use synapse_libsql::Store as LibsqlStore;
use synapsql::server::brain_adapter::BrainAdapter;

fn tmp_path() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CTR: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir();
    let pid = std::process::id();
    let seq = CTR.fetch_add(1, Ordering::Relaxed);
    let name = format!("synapsql_e2e_{}_{}.db", pid, seq);
    dir.join(name).to_string_lossy().to_string()
}

#[tokio::test]
async fn select_1() {
    let path = tmp_path();
    let adapter = BrainAdapter::open(&path).unwrap();
    let res = adapter.query("SELECT 1").await.unwrap();
    assert_eq!(res.rows.len(), 1);
    assert!(String::from_utf8_lossy(&res.rows[0]).contains('1'));
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn create_insert_select() {
    let path = tmp_path();
    let adapter = BrainAdapter::open(&path).unwrap();

    // Use a table name that doesn't conflict with synapse-core migrations.
    adapter.exec("CREATE TABLE test_items (id INTEGER PRIMARY KEY, label TEXT)").await.unwrap();
    adapter.exec("INSERT INTO test_items VALUES (1, 'rust')").await.unwrap();
    adapter.exec("INSERT INTO test_items VALUES (2, 'python')").await.unwrap();

    let res = adapter.query("SELECT id, label FROM test_items ORDER BY id").await.unwrap();
    assert_eq!(res.rows.len(), 2);
    assert!(String::from_utf8_lossy(&res.rows[0]).contains("rust"));
    assert!(String::from_utf8_lossy(&res.rows[1]).contains("python"));

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn fts_routed_via_match_against() {
    let path = tmp_path();
    let adapter = BrainAdapter::open(&path).unwrap();

    // Prime the synapse-core store with a doc so FTS can find it.
    {
        use synapse_libsql::Store as _;
        // We use exec to also exercise the exec path; for FTS we need the
        // synapse-core FTS5 virtual table which is initialised by Store::open.
        // Insert a real doc via the LibsqlStore exec path (raw SQL on synapse tables).
        let _ = adapter.exec(
            "INSERT OR IGNORE INTO memories (text, embedding) VALUES ('rust is fast', zeroblob(0))"
        ).await; // may fail if table has different schema — that's fine for smoke
    }

    // MATCH…AGAINST → FtsSearch extension → search_lex on synapse-core.
    // Even if no hits, the call must not error out.
    let res = adapter.query(
        "SELECT * FROM docs WHERE MATCH(text) AGAINST ('rust')"
    ).await.unwrap();
    // We care about no panic/error; hit count may be 0.
    let _ = res;

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn vec_search_returns_wire_error() {
    let path = tmp_path();
    let adapter = BrainAdapter::open(&path).unwrap();

    // `<=>` operator → VecSearch extension → wire error (embed pipeline not wired).
    let res = adapter.query(
        "SELECT * FROM docs WHERE embedding <=> ARRAY[0.1,0.2] < 0.3"
    ).await;
    assert!(res.is_err(), "vec-search must return wire error, not fake-OK stub");
    let err_msg = res.unwrap_err().to_string();
    assert!(err_msg.contains("embedding pipeline not wired"), "unexpected error: {err_msg}");

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn shared_adapter_sequential_inserts() {
    let path = tmp_path();
    let adapter = Arc::new(BrainAdapter::open(&path).unwrap());
    adapter.exec("CREATE TABLE t (v INTEGER)").await.unwrap();

    // Sequential inserts via the same Arc — validates shared ownership works.
    for i in 0..8i64 {
        adapter.exec(&format!("INSERT INTO t VALUES ({i})")).await.unwrap();
    }

    let res = adapter.query("SELECT COUNT(*) FROM t").await.unwrap();
    assert_eq!(res.rows.len(), 1);
    assert!(String::from_utf8_lossy(&res.rows[0]).contains('8'));

    let _ = std::fs::remove_file(&path);
}
