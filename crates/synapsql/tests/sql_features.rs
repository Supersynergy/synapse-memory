//! Integration tests for TOP-5 missing SQL features.
//!
//! Feature 1+5: EXPLAIN / EXPLAIN ANALYZE (vector-aware planner)
//! Feature 2:   BEGIN / COMMIT / ROLLBACK transaction parsing
//! Feature 3:   Window functions — ROW_NUMBER, RANK (SQLite passthrough verify)
//! Feature 4:   JSON_EXTRACT / JSON_SET (SQLite json1 passthrough verify)

// ── Feature 1+5: EXPLAIN ──────────────────────────────────────────────────────

#[cfg(test)]
mod explain_tests {
    use synapsql::sql_ext::explain::{explain_plan, strip_explain};

    #[test]
    fn explain_plain_select() {
        let rows = explain_plan("SELECT * FROM users WHERE id = 1", false);
        assert!(!rows.is_empty());
        let ops: Vec<&str> = rows.iter().map(|r| r.op.as_str()).collect();
        assert!(ops.contains(&"SQLite"), "expected SQLite op, got {:?}", ops);
    }

    #[test]
    fn explain_analyze_flag() {
        let (inner, analyze) = strip_explain("EXPLAIN ANALYZE SELECT id FROM docs LIMIT 5").unwrap();
        assert!(analyze);
        assert_eq!(inner, "SELECT id FROM docs LIMIT 5");
    }

    #[test]
    fn explain_query_plan_compat() {
        let (inner, analyze) = strip_explain("EXPLAIN QUERY PLAN SELECT * FROM t").unwrap();
        assert!(!analyze);
        assert_eq!(inner, "SELECT * FROM t");
    }

    #[test]
    fn explain_vec_shows_hnsw() {
        let rows = explain_plan("SELECT id FROM docs WHERE embedding <=> :q LIMIT 10", false);
        let ops: Vec<&str> = rows.iter().map(|r| r.op.as_str()).collect();
        assert!(ops.contains(&"HNSWIndexScan"), "ops: {:?}", ops);
    }

    #[test]
    fn explain_fts_shows_fts5() {
        let rows = explain_plan("SELECT id FROM docs WHERE MATCH(body) AGAINST (:q)", false);
        let ops: Vec<&str> = rows.iter().map(|r| r.op.as_str()).collect();
        assert!(ops.contains(&"FTS5IndexScan"), "ops: {:?}", ops);
    }

    #[test]
    fn explain_hybrid_shows_rrf() {
        let sql = "SELECT HYBRID_RANK(body, emb, :q) AS s FROM docs ORDER BY s DESC";
        let rows = explain_plan(sql, false);
        let ops: Vec<&str> = rows.iter().map(|r| r.op.as_str()).collect();
        assert!(ops.contains(&"HybridRRFPlan"), "ops: {:?}", ops);
        assert!(ops.contains(&"RRFFusion"), "ops: {:?}", ops);
        assert!(ops.contains(&"HNSWIndexScan"), "ops: {:?}", ops);
        assert!(ops.contains(&"FTS5IndexScan"), "ops: {:?}", ops);
    }

    #[test]
    fn explain_cost_field_present() {
        let rows = explain_plan("SELECT 1", false);
        for row in &rows {
            assert!(!row.estimated_cost.is_empty());
        }
    }

    #[test]
    fn strip_explain_bare_returns_none() {
        assert!(strip_explain("SELECT 1").is_none());
        assert!(strip_explain("EXPLAIN").is_none());
    }
}

// ── Feature 2: Transactions ───────────────────────────────────────────────────

#[cfg(test)]
mod txn_tests {
    use synapsql::parser::{classify_txn, is_txn_statement, TxnStatement};

    #[test]
    fn begin_plain() {
        assert_eq!(classify_txn("BEGIN"), Some(TxnStatement::Begin));
    }

    #[test]
    fn begin_with_semicolon() {
        assert_eq!(classify_txn("begin;"), Some(TxnStatement::Begin));
    }

    #[test]
    fn start_transaction() {
        assert_eq!(classify_txn("START TRANSACTION"), Some(TxnStatement::Begin));
    }

    #[test]
    fn commit_plain() {
        assert_eq!(classify_txn("COMMIT"), Some(TxnStatement::Commit));
    }

    #[test]
    fn end_alias() {
        assert_eq!(classify_txn("END"), Some(TxnStatement::Commit));
    }

    #[test]
    fn rollback_plain() {
        assert_eq!(classify_txn("ROLLBACK"), Some(TxnStatement::Rollback));
    }

    #[test]
    fn savepoint() {
        assert_eq!(
            classify_txn("SAVEPOINT sp1"),
            Some(TxnStatement::Savepoint("sp1".into()))
        );
    }

    #[test]
    fn release_savepoint() {
        assert_eq!(
            classify_txn("RELEASE SAVEPOINT sp1"),
            Some(TxnStatement::ReleaseSavepoint("sp1".into()))
        );
    }

    #[test]
    fn select_not_txn() {
        assert!(!is_txn_statement("SELECT 1"));
    }

    #[test]
    fn insert_not_txn() {
        assert!(!is_txn_statement("INSERT INTO t VALUES (1)"));
    }
}

// ── Feature 3: Window functions (SQLite passthrough rewriter check) ───────────

#[cfg(test)]
mod window_function_tests {
    use synapsql::parser::rewrite;

    /// Window functions use no SynapsQL extensions → rewriter must pass SQL through unchanged.
    #[test]
    fn row_number_passthrough() {
        let sql = "SELECT *, ROW_NUMBER() OVER (PARTITION BY dept ORDER BY salary DESC) AS rn FROM employees";
        let result = rewrite(sql);
        // No extensions detected
        assert!(result.extensions.is_empty(), "unexpected extensions: {:?}", result.extensions);
        // SQL unchanged (window function intact)
        assert!(result.sql.contains("ROW_NUMBER()"), "sql modified: {}", result.sql);
        assert!(result.sql.contains("OVER"), "sql modified: {}", result.sql);
    }

    #[test]
    fn rank_passthrough() {
        let sql = "SELECT *, RANK() OVER (ORDER BY score DESC) AS r FROM scores";
        let result = rewrite(sql);
        assert!(result.extensions.is_empty());
        assert!(result.sql.contains("RANK()"));
    }

    #[test]
    fn dense_rank_passthrough() {
        let sql = "SELECT *, DENSE_RANK() OVER (PARTITION BY cat ORDER BY val) AS dr FROM t";
        let result = rewrite(sql);
        assert!(result.extensions.is_empty());
        assert!(result.sql.contains("DENSE_RANK()"));
    }

    #[test]
    fn lag_lead_passthrough() {
        let sql = "SELECT id, LAG(val, 1) OVER (ORDER BY ts) AS prev FROM series";
        let result = rewrite(sql);
        assert!(result.extensions.is_empty());
        assert!(result.sql.contains("LAG("));
    }

    #[test]
    fn ntile_passthrough() {
        let sql = "SELECT id, NTILE(4) OVER (ORDER BY score) AS quartile FROM scores";
        let result = rewrite(sql);
        assert!(result.extensions.is_empty());
        assert!(result.sql.contains("NTILE(4)"));
    }
}

// ── Feature 4: JSON functions (SQLite json1 passthrough) ─────────────────────

#[cfg(test)]
mod json_function_tests {
    use synapsql::parser::rewrite;

    #[test]
    fn json_extract_passthrough() {
        let sql = "SELECT JSON_EXTRACT(meta, '$.key') FROM docs WHERE id = 1";
        let result = rewrite(sql);
        assert!(result.extensions.is_empty());
        assert!(result.sql.contains("JSON_EXTRACT"), "sql: {}", result.sql);
    }

    #[test]
    fn json_set_passthrough() {
        let sql = "UPDATE docs SET meta = JSON_SET(meta, '$.count', 42) WHERE id = 1";
        let result = rewrite(sql);
        assert!(result.extensions.is_empty());
        assert!(result.sql.contains("JSON_SET"), "sql: {}", result.sql);
    }

    #[test]
    fn json_object_passthrough() {
        let sql = "SELECT JSON_OBJECT('name', name, 'age', age) FROM users";
        let result = rewrite(sql);
        assert!(result.extensions.is_empty());
        assert!(result.sql.contains("JSON_OBJECT"), "sql: {}", result.sql);
    }

    #[test]
    fn json_array_passthrough() {
        let sql = "SELECT JSON_ARRAY(1, 2, 3)";
        let result = rewrite(sql);
        assert!(result.extensions.is_empty());
        assert!(result.sql.contains("JSON_ARRAY"), "sql: {}", result.sql);
    }

    #[test]
    fn json_arrow_operator_passthrough() {
        // SQLite 3.38+ supports -> and ->> operators
        let sql = "SELECT meta -> '$.name' FROM docs";
        let result = rewrite(sql);
        // Rewriter must not accidentally detect -> as <=> or any other extension
        assert!(result.sql.contains("->"), "sql: {}", result.sql);
    }
}
