//! Query rewriter — transforms Synapse SQL extensions into executable plans.
//!
//! Extensions handled:
//!   1. `WHERE col <=> :param < threshold` → VecSearch plan
//!   2. `MATCH(col) AGAINST (expr)` → FTS/BM25 plan
//!   3. `HYBRID_RANK(...)` → RRF fusion plan
//!   4. `WITH RECALL_GUARANTEE alpha` → conformal wrapper
//!
//! Optimizations:
//!   - Predicate Pushdown: scalar `col = val` / `col > val` / `col < val` predicates
//!     extracted from mixed WHERE clauses so they run BEFORE the vec-search.
//!     Pattern: TanStack/db optimizer.ts + Apache Hive PredicatePushDown.java
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
    /// Predicate pushdown: scalar filters extracted from mixed WHERE clause.
    /// These run on metadata BEFORE the vec-search to shrink the candidate set.
    PredicatePushdown { predicates: Vec<ScalarPredicate> },
}

/// A single scalar predicate extracted via predicate pushdown.
#[derive(Debug, Clone, PartialEq)]
pub struct ScalarPredicate {
    pub column: String,
    pub op: PredicateOp,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PredicateOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
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

    // 3. Vec operator `<=>` — with predicate pushdown
    if let Some(op) = VectorOp::parse(&working) {
        // Extract scalar predicates before we stub out the WHERE clause.
        // These fire first (metadata filter), shrinking the ANN candidate set.
        let pushed = extract_scalar_predicates(&working);
        if !pushed.is_empty() {
            extensions.push(Extension::PredicatePushdown { predicates: pushed });
        }
        extensions.push(Extension::VecSearch(op));
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

/// Extract scalar predicates from a WHERE clause that also contains `<=>`.
///
/// Pattern (Hive/TanStack): scan tokens between WHERE and the vec operator,
/// pick `col OP literal` triples where OP ∈ {=,!=,<,<=,>,>=}.
/// These predicates are emitted as `PredicatePushdown` so the executor can
/// apply a cheap metadata filter BEFORE running the expensive ANN search.
pub fn extract_scalar_predicates(sql: &str) -> Vec<ScalarPredicate> {
    let upper = sql.to_ascii_uppercase();
    let where_pos = match upper.find("WHERE") {
        Some(p) => p + 5,
        None => return vec![],
    };
    // Only look before the `<=>` operator
    let vec_pos = upper.find("<=>").unwrap_or(sql.len());
    if vec_pos <= where_pos { return vec![]; }
    let clause = &sql[where_pos..vec_pos];

    let mut results = Vec::new();
    // Split by AND (case-insensitive); ignore OR (too risky to push through OR)
    for part in clause.split_ascii_whitespace_and_and(clause) {
        let part = part.trim();
        if part.is_empty() { continue; }
        let part_upper = part.to_ascii_uppercase();
        // Try operators longest-first to avoid `<` matching `<=`
        let ops: &[(&str, PredicateOp)] = &[
            ("!=", PredicateOp::Ne),
            ("<>", PredicateOp::Ne),
            ("<=", PredicateOp::Le),
            (">=", PredicateOp::Ge),
            ("<",  PredicateOp::Lt),
            (">",  PredicateOp::Gt),
            ("=",  PredicateOp::Eq),
        ];
        for (sym, op) in ops {
            if let Some(pos) = part.find(sym) {
                // Skip if part of `<=>` (vec operator leaking through)
                if sym == &"<" || sym == &">" {
                    let next = part.as_bytes().get(pos + sym.len()).copied();
                    if next == Some(b'=') || next == Some(b'>') { continue; }
                }
                let col = part[..pos].trim().to_owned();
                let val = part[pos + sym.len()..].trim().to_owned();
                // col must be a simple identifier, val a literal or :param
                let col_ok = col.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.');
                let val_ok = !val.is_empty() && !val.to_ascii_uppercase().contains("SELECT");
                if col_ok && val_ok {
                    results.push(ScalarPredicate { column: col, op: op.clone(), value: val });
                }
                break;
            }
        }
        let _ = part_upper;
    }
    results
}

/// Split a WHERE fragment by AND tokens (case-insensitive).
trait SplitByAnd {
    fn split_ascii_whitespace_and_and<'a>(&'a self, s: &'a str) -> Vec<&'a str>;
}

impl SplitByAnd for str {
    fn split_ascii_whitespace_and_and<'a>(&'a self, s: &'a str) -> Vec<&'a str> {
        let upper = s.to_ascii_uppercase();
        let mut parts = Vec::new();
        let mut start = 0;
        let bytes = upper.as_bytes();
        let len = bytes.len();
        let mut i = 0;
        while i + 3 <= len {
            if &bytes[i..i+3] == b"AND" {
                let prev_ok = i == 0 || bytes[i-1].is_ascii_whitespace();
                let next_ok = i + 3 >= len || bytes[i+3].is_ascii_whitespace();
                if prev_ok && next_ok {
                    parts.push(s[start..i].trim());
                    start = i + 3;
                    i += 3;
                    continue;
                }
            }
            i += 1;
        }
        parts.push(s[start..].trim());
        parts
    }
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
    fn predicate_pushdown_extracts_scalar() {
        let sql = "SELECT id FROM docs WHERE tenant_id = 'acme' AND embedding <=> :q LIMIT 10";
        let r = rewrite(sql);
        let pushed = r.extensions.iter().find_map(|e| {
            if let Extension::PredicatePushdown { predicates } = e { Some(predicates) } else { None }
        });
        assert!(pushed.is_some(), "expected PredicatePushdown extension");
        let preds = pushed.unwrap();
        assert_eq!(preds.len(), 1);
        assert_eq!(preds[0].column, "tenant_id");
        assert_eq!(preds[0].op, PredicateOp::Eq);
        assert_eq!(preds[0].value, "'acme'");
    }

    #[test]
    fn predicate_pushdown_multiple() {
        let sql = "SELECT id FROM docs WHERE score > 0.5 AND lang = 'en' AND emb <=> :q LIMIT 5";
        let r = rewrite(sql);
        let pushed = r.extensions.iter().find_map(|e| {
            if let Extension::PredicatePushdown { predicates } = e { Some(predicates) } else { None }
        });
        assert!(pushed.is_some());
        let preds = pushed.unwrap();
        assert_eq!(preds.len(), 2);
    }

    #[test]
    fn no_pushdown_without_vec() {
        let sql = "SELECT id FROM docs WHERE tenant_id = 'acme' LIMIT 10";
        let r = rewrite(sql);
        assert!(!r.extensions.iter().any(|e| matches!(e, Extension::PredicatePushdown { .. })));
    }

    #[test]
    fn conformal_recall() {
        let sql = "SELECT id FROM docs WHERE embedding <=> :q LIMIT 10 WITH RECALL_GUARANTEE 0.99";
        let r = rewrite(sql);
        assert!(r.extensions.iter().any(|e| matches!(e, Extension::ConformalRecall { alpha } if *alpha == 0.99)));
    }
}
