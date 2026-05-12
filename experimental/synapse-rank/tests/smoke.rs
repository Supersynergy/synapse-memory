use synapse_learn::query_log::{QueryEvent, QueryLog};
use tempfile::NamedTempFile;
use std::time::{SystemTime, UNIX_EPOCH};

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

#[test]
fn test_log_and_click() {
    let f = NamedTempFile::new().unwrap();
    let ql = QueryLog::open(f.path()).unwrap();

    let id = ql.log_event(&QueryEvent {
        ts: now(),
        query_text: "rust async".into(),
        query_embed: None,
        result_doc_id: 42,
        rank: 1,
        score: 0.95,
        bm25_score: Some(12.3),
        vec_score: Some(0.88),
        session_id: Some("s1".into()),
    }).unwrap();

    assert!(id > 0);
    ql.mark_click(id, 1500).unwrap();
}

#[test]
fn test_export_libsvm() {
    let f = NamedTempFile::new().unwrap();
    let ql = QueryLog::open(f.path()).unwrap();

    for i in 0..3i64 {
        ql.log_event(&QueryEvent {
            ts: now(),
            query_text: "test query".into(),
            query_embed: None,
            result_doc_id: i,
            rank: (i + 1) as i32,
            score: 1.0 - (i as f64 * 0.1),
            bm25_score: Some(10.0 - i as f64),
            vec_score: Some(0.9 - i as f64 * 0.05),
            session_id: None,
        }).unwrap();
    }

    // mark first as clicked
    ql.mark_click(1, 2000).unwrap();

    let out = NamedTempFile::new().unwrap();
    let n = ql.export_libsvm(out.path()).unwrap();
    assert_eq!(n, 3);

    let content = std::fs::read_to_string(out.path()).unwrap();
    assert!(content.contains("qid:1"));
    // first row should be clicked=1
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].starts_with("1 ") || lines[1].starts_with("1 ") || lines[2].starts_with("1 "));
}
