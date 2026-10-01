#[cfg(feature = "olap")]
mod olap_tests {
    use rusqlite::Connection as SqliteConn;
    use synapse_olap::OlapEngine;
    use tempfile::NamedTempFile;

    fn make_sqlite_db(n: usize) -> NamedTempFile {
        let f = NamedTempFile::new().unwrap();
        let conn = SqliteConn::open(f.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE events (id INTEGER PRIMARY KEY, category TEXT, value REAL);",
        )
        .unwrap();
        let mut stmt = conn.prepare("INSERT INTO events VALUES (?,?,?)").unwrap();
        for i in 0..n {
            let id = i as i64;
            let cat = format!("cat{}", i % 50);
            let val = (i % 1000) as f64;
            stmt.execute(rusqlite::params![id, cat, val]).unwrap();
        }
        f
    }

    #[test]
    fn test_memory_basic() {
        let mut eng = OlapEngine::open_memory().unwrap();
        eng.execute("CREATE TABLE t (x INTEGER)").unwrap();
        eng.execute("INSERT INTO t SELECT range FROM range(100)")
            .unwrap();
        let batches = eng.query("SELECT COUNT(*) as cnt FROM t").unwrap();
        assert!(!batches.is_empty());
    }

    #[test]
    fn test_attach_sqlite_group_by() {
        let db = make_sqlite_db(10_000);
        let mut eng = OlapEngine::open_memory().unwrap();
        eng.attach_synapse_db(db.path(), "synx").unwrap();
        let batches = eng
            .query(
                "SELECT category, COUNT(*) as cnt, AVG(value) as avg_val \
                    FROM synx.events GROUP BY category ORDER BY category",
            )
            .unwrap();
        assert!(!batches.is_empty());
        // 50 distinct categories
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 50);
    }

    #[test]
    fn test_group_by_1m_faster_indication() {
        // smoke: 1M rows GROUP BY completes without error
        // (benchmark numbers in bench/olap_groupby — this is correctness only)
        let db = make_sqlite_db(1_000_000);
        let mut eng = OlapEngine::open_memory().unwrap();
        eng.attach_synapse_db(db.path(), "synx").unwrap();
        let batches = eng
            .query("SELECT category, SUM(value) FROM synx.events GROUP BY category")
            .unwrap();
        let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(rows, 50);
    }
}

// Router tests always available (no feature gate)
mod router_tests {
    use synapse_olap::{Engine, auto_route, is_olap};

    #[test]
    fn oltp_point_lookup() {
        assert!(!is_olap("SELECT * FROM docs WHERE id = 42"));
        assert_eq!(auto_route("SELECT * FROM docs WHERE id = 42"), Engine::Oltp);
    }

    #[test]
    fn olap_count() {
        assert!(is_olap("SELECT COUNT(*) FROM docs"));
    }

    #[test]
    fn olap_group_by() {
        assert!(is_olap(
            "SELECT category, SUM(value) FROM events GROUP BY category"
        ));
        assert_eq!(
            auto_route("SELECT category, SUM(value) FROM events GROUP BY category"),
            Engine::Olap
        );
    }

    #[test]
    fn olap_window() {
        assert!(is_olap(
            "SELECT id, ROW_NUMBER() OVER (PARTITION BY cat ORDER BY ts) FROM t"
        ));
    }

    #[test]
    fn olap_distinct() {
        assert!(is_olap("SELECT DISTINCT category FROM events"));
    }

    #[test]
    fn oltp_insert() {
        assert!(!is_olap("INSERT INTO docs (id, text) VALUES (1, 'hello')"));
    }
}
