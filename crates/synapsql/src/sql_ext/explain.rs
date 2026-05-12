//! EXPLAIN / EXPLAIN ANALYZE plan builder for SynapsQL.
//!
//! Detects extension-aware query plans:
//!   - Vector `<=>` → "HNSW index scan"
//!   - MATCH ... AGAINST → "FTS5/Tantivy index scan"
//!   - HYBRID_RANK → "Hybrid RRF plan (HNSW + FTS)"
//!   - Plain SQL → "SQLite EXPLAIN passthrough"

use crate::parser::rewrite;
use crate::parser::rewriter::Extension;

/// A single row in an EXPLAIN plan output.
#[derive(Debug, Clone)]
pub struct PlanRow {
    pub step: u32,
    pub op: String,
    pub detail: String,
    pub estimated_cost: String,
}

impl PlanRow {
    fn new(step: u32, op: &str, detail: &str, cost: &str) -> Self {
        Self {
            step,
            op: op.to_owned(),
            detail: detail.to_owned(),
            estimated_cost: cost.to_owned(),
        }
    }
}

/// Column names for EXPLAIN output.
pub const EXPLAIN_COLS: &[&str] = &["step", "op", "detail", "estimated_cost"];

/// Build an EXPLAIN plan for `sql`.
///
/// If `analyze` is true, the label shows EXPLAIN ANALYZE.
/// Returns rows suitable for wire-protocol text response.
pub fn explain_plan(sql: &str, analyze: bool) -> Vec<PlanRow> {
    let label = if analyze {
        "EXPLAIN ANALYZE"
    } else {
        "EXPLAIN"
    };
    let result = rewrite(sql);
    let mut rows: Vec<PlanRow> = Vec::new();
    let mut step = 0u32;

    // Parse phase annotation
    rows.push(PlanRow::new(
        step,
        label,
        &format!("input: {}", sql.trim()),
        "0",
    ));
    step += 1;

    if result.extensions.is_empty() {
        // Plain SQL — delegate to SQLite EXPLAIN
        rows.push(PlanRow::new(
            step,
            "SQLite",
            "passthrough to SQLite query planner",
            "~1",
        ));
        step += 1;
        rows.push(PlanRow::new(
            step,
            "IndexScan",
            "SQLite chooses index via cost-based optimizer",
            "varies",
        ));
        return rows;
    }

    for ext in &result.extensions {
        match ext {
            Extension::VecSearch(op) => {
                rows.push(PlanRow::new(
                    step,
                    "HNSWIndexScan",
                    &format!("col={} param={} k={}", op.column, op.param, op.k),
                    "O(log N)",
                ));
                step += 1;
                rows.push(PlanRow::new(
                    step,
                    "ANN",
                    "approximate nearest-neighbour via HNSW (SimSIMD kernels)",
                    "~8ms/113k",
                ));
            }
            Extension::FtsSearch {
                column,
                query_param,
            } => {
                rows.push(PlanRow::new(
                    step,
                    "FTS5IndexScan",
                    &format!("col={} param={}", column, query_param),
                    "O(log N)",
                ));
                step += 1;
                rows.push(PlanRow::new(
                    step,
                    "BM25Rank",
                    "FTS5 BM25 scoring (Tantivy-compatible)",
                    "~2ms/100k",
                ));
            }
            Extension::HybridRank {
                text_col,
                vec_col,
                query_param,
            } => {
                rows.push(PlanRow::new(
                    step,
                    "HybridRRFPlan",
                    &format!(
                        "text_col={} vec_col={} param={} fusion=RRF",
                        text_col, vec_col, query_param
                    ),
                    "O(log N)",
                ));
                step += 1;
                rows.push(PlanRow::new(
                    step,
                    "HNSWIndexScan",
                    &format!("col={} HNSW arm", vec_col),
                    "~8ms",
                ));
                step += 1;
                rows.push(PlanRow::new(
                    step,
                    "FTS5IndexScan",
                    &format!("col={} BM25 arm", text_col),
                    "~2ms",
                ));
                step += 1;
                rows.push(PlanRow::new(
                    step,
                    "RRFFusion",
                    "Reciprocal Rank Fusion k=60",
                    "O(n_results)",
                ));
            }
            Extension::ConformalRecall { alpha } => {
                rows.push(PlanRow::new(
                    step,
                    "ConformalWrapper",
                    &format!("recall_guarantee alpha={}", alpha),
                    "adaptive k",
                ));
            }
            Extension::PredicatePushdown { predicates } => {
                rows.push(PlanRow::new(
                    step,
                    "PredicatePushdown",
                    &format!(
                        "{} scalar filters pushed before vec-search",
                        predicates.len()
                    ),
                    "O(candidates)",
                ));
            }
        }
        step += 1;
    }

    rows
}

/// Returns true if `sql` is an EXPLAIN or EXPLAIN ANALYZE statement.
/// Strips the prefix and returns the inner SQL + analyze flag.
pub fn strip_explain(sql: &str) -> Option<(String, bool)> {
    let trimmed = sql.trim();
    let upper = trimmed.to_ascii_uppercase();
    if upper.starts_with("EXPLAIN") {
        let rest = trimmed[7..].trim_start();
        let upper_rest = rest.to_ascii_uppercase();
        let (inner, analyze) = if upper_rest.starts_with("ANALYZE") {
            (rest[7..].trim().to_owned(), true)
        } else if upper_rest.starts_with("QUERY PLAN") {
            // SQLite EXPLAIN QUERY PLAN
            (rest[10..].trim().to_owned(), false)
        } else {
            (rest.to_owned(), false)
        };
        if !inner.is_empty() {
            return Some((inner, analyze));
        }
        // bare EXPLAIN with no inner SQL → not a query
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explain_vec_plan() {
        let rows = explain_plan("SELECT id FROM docs WHERE embedding <=> :q LIMIT 10", false);
        let ops: Vec<&str> = rows.iter().map(|r| r.op.as_str()).collect();
        assert!(ops.contains(&"HNSWIndexScan"), "ops: {:?}", ops);
    }

    #[test]
    fn explain_fts_plan() {
        let rows = explain_plan("SELECT id FROM docs WHERE MATCH(body) AGAINST (:q)", false);
        let ops: Vec<&str> = rows.iter().map(|r| r.op.as_str()).collect();
        assert!(ops.contains(&"FTS5IndexScan"), "ops: {:?}", ops);
    }

    #[test]
    fn explain_hybrid_plan() {
        let sql = "SELECT id, HYBRID_RANK(body, emb, :q) AS s FROM docs ORDER BY s DESC";
        let rows = explain_plan(sql, false);
        let ops: Vec<&str> = rows.iter().map(|r| r.op.as_str()).collect();
        assert!(ops.contains(&"HybridRRFPlan"), "ops: {:?}", ops);
        assert!(ops.contains(&"RRFFusion"), "ops: {:?}", ops);
    }

    #[test]
    fn explain_plain_sql() {
        let rows = explain_plan("SELECT * FROM users WHERE id = 1", false);
        let ops: Vec<&str> = rows.iter().map(|r| r.op.as_str()).collect();
        assert!(ops.contains(&"SQLite"), "ops: {:?}", ops);
    }

    #[test]
    fn strip_explain_basic() {
        let (inner, analyze) = strip_explain("EXPLAIN SELECT 1").unwrap();
        assert_eq!(inner, "SELECT 1");
        assert!(!analyze);
    }

    #[test]
    fn strip_explain_analyze() {
        let (inner, analyze) = strip_explain("EXPLAIN ANALYZE SELECT id FROM t LIMIT 5").unwrap();
        assert_eq!(inner, "SELECT id FROM t LIMIT 5");
        assert!(analyze);
    }

    #[test]
    fn strip_explain_query_plan() {
        let (inner, _) = strip_explain("EXPLAIN QUERY PLAN SELECT * FROM t").unwrap();
        assert_eq!(inner, "SELECT * FROM t");
    }
}
