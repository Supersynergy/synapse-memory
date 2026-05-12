//! Query rewriter — transforms Synapse SQL extensions into executable plans.
//!
//! Extensions handled:
//!   1. `WHERE col <=> :param < threshold` → VecSearch plan
//!   2. `MATCH(col) AGAINST (expr)` → FTS/BM25 plan
//!   3. `HYBRID_RANK(...)` → RRF fusion plan
//!   4. `WITH RECALL_GUARANTEE alpha` → conformal wrapper
//!
//! Architecture stolen from Vitess: AST rewrite before backend dispatch.

use crate::sql_ext::{
    vector_op::VectorOp,
    conformal::strip_recall_clause,
};

/// The result of a rewrite pass.
#[derive(Debug, Clone)]
pub struct RewriteResult {
    /// SQL to send to the backend (extensions stripped/translated).
    pub sql: String,
    /// Detected extensions that need special execution.
    pub extensions: Vec<Extension>,
}

/// A detected SQL extension.
#[derive(Debug, Clone)]
pub enum Extension {
    /// Vector ANN search: `col <=> :param LIMIT k`.
    VecSearch(VectorOp),
    /// Full-text search: `MATCH(col) AGAINST(expr)`.
    FtsSearch { column: String, query_param: String },
    /// Hybrid RRF: combine vec + FTS.
    HybridRank { text_col: String, vec_col: String, query_param: String },
    /// Conformal recall guarantee.
    ConformalRecall { alpha: f64 },
}

/// Rewrite `sql`, stripping/translating all extensions.
/// Returns the backend-safe SQL + list of extensions to apply.
pub fn rewrite(sql: &str) -> RewriteResult {
    let mut extensions = Vec::new();
    let mut working = sql.to_owned();

    // 1. Strip conformal clause
    let (clean, alpha) = strip_recall_clause(&working);
    if let Some(a) = alpha {
        extensions.push(Extension::ConformalRecall { alpha: a });
        working = clean.to_owned();
    }

    // 2. Detect HYBRID_RANK — must come before vec/fts so we don't double-detect
    if let Some(hr) = parse_hybrid_rank(&working) {
        extensions.push(Extension::HybridRank {
            text_col: hr.0,
            vec_col: hr.1,
            query_param: hr.2,
        });
        // Leave SQL as-is — backend will see HYBRID_RANK call and we intercept
        return RewriteResult { sql: working, extensions };
    }

    // 3. Vec operator `<=>`
    if let Some(op) = VectorOp::parse(&working) {
        extensions.push(Extension::VecSearch(op));
        // Rewrite to a passthrough SELECT — backend executes ANN, not raw SQL
        working = rewrite_vec_to_stub(&working);
    }

    // 4. MATCH ... AGAINST
    if let Some((col, param)) = parse_match_against(&working) {
        extensions.push(Extension::FtsSearch { column: col, query_param: param });
        working = rewrite_match_to_stub(&working);
    }

    RewriteResult { sql: working, extensions }
}

/// Detect `HYBRID_RANK(text_col, vec_col, :param)`.
fn parse_hybrid_rank(sql: &str) -> Option<(String, String, String)> {
    let upper = sql.to_ascii_uppercase();
    let pos = upper.find("HYBRID_RANK(")?;
    let args_str = &sql[pos + "HYBRID_RANK(".len()..];
    let close = args_str.find(')')?;
    let args: Vec<&str> = args_str[..close].split(',').collect();
    if args.len() >= 3 {
        Some((
            args[0].trim().to_owned(),
            args[1].trim().to_owned(),
            args[2].trim().trim_start_matches(':').to_owned(),
        ))
    } else {
        None
    }
}

/// Detect `MATCH(col) AGAINST (:param)`.
fn parse_match_against(sql: &str) -> Option<(String, String)> {
    let upper = sql.to_ascii_uppercase();
    let match_pos = upper.find("MATCH(")?;
    let col_start = match_pos + 6;
    let col_end = sql[col_start..].find(')')? + col_start;
    let col = sql[col_start..col_end].trim().to_owned();

    let against_pos = upper[col_end..].find("AGAINST")? + col_end;
    let after = &sql[against_pos + 7..];
    let open = after.find('(')?;
    let close = after.find(')')?;
    let param = after[open + 1..close]
        .trim()
        .trim_start_matches(':')
        .to_owned();

    Some((col, param))
}

/// Replace `<=> :param [< threshold]` with a placeholder that backends ignore.
fn rewrite_vec_to_stub(sql: &str) -> String {
    // Strip WHERE clause containing <=> down to `WHERE 1=1`
    // Simple approach: replace everything from WHERE...LIMIT with WHERE 1=1 LIMIT
    if let Some(where_pos) = sql.to_ascii_uppercase().find("WHERE") {
        let limit_pos = sql.to_ascii_uppercase().find("LIMIT");
        let prefix = &sql[..where_pos];
        let suffix = limit_pos.map(|p| &sql[p..]).unwrap_or("");
        format!("{}WHERE 1=1 {}", prefix, suffix)
    } else {
        sql.to_owned()
    }
}

/// Replace `MATCH(...) AGAINST(...)` with `1=1`.
fn rewrite_match_to_stub(sql: &str) -> String {
    let upper = sql.to_ascii_uppercase();
    if let Some(match_pos) = upper.find("MATCH(") {
        // Find the closing AGAINST(...)
        if let Some(against_end) = upper[match_pos..].find("AGAINST") {
            let abs_against = match_pos + against_end;
            if let Some(close) = sql[abs_against..].find(')') {
                let end = abs_against + close + 1;
                return format!("{}1=1{}", &sql[..match_pos], &sql[end..]);
            }
        }
    }
    sql.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vec_rewrite() {
        let sql = "SELECT id FROM docs WHERE embedding <=> :q LIMIT 10";
        let r = rewrite(sql);
        assert!(r.extensions.iter().any(|e| matches!(e, Extension::VecSearch(_))));
        assert!(!r.sql.contains("<=>"), "stub sql: {}", r.sql);
    }

    #[test]
    fn hybrid_rank_detected() {
        let sql = "SELECT id, HYBRID_RANK(body, emb, :q) AS s FROM docs ORDER BY s DESC";
        let r = rewrite(sql);
        assert!(r.extensions.iter().any(|e| matches!(e, Extension::HybridRank { .. })));
    }

    #[test]
    fn conformal_recall() {
        let sql = "SELECT id FROM docs WHERE embedding <=> :q LIMIT 10 WITH RECALL_GUARANTEE 0.99";
        let r = rewrite(sql);
        assert!(r.extensions.iter().any(|e| matches!(e, Extension::ConformalRecall { alpha } if *alpha == 0.99)));
    }
}
