//! `WITH RECALL_GUARANTEE <alpha>` clause — conformal recall wrapper.
//!
//! Strips the trailing clause from SQL before forwarding to the backend,
//! then passes `alpha` to synapse-core `SearchOptions.conformal_target`.
//!
//! # Syntax
//! ```sql
//! SELECT id FROM docs WHERE embedding <=> :q LIMIT 10
//! WITH RECALL_GUARANTEE 0.99;
//! ```
//!
//! TODO: wire parsed `alpha` into synapse-core conformal_search.

/// Parse and strip a trailing `WITH RECALL_GUARANTEE <alpha>` clause.
/// Returns `(clean_sql, Some(alpha))` or `(original_sql, None)`.
pub fn strip_recall_clause(sql: &str) -> (&str, Option<f64>) {
    let upper = sql.to_ascii_uppercase();
    if let Some(pos) = upper.rfind("WITH RECALL_GUARANTEE") {
        let after = sql[pos + "WITH RECALL_GUARANTEE".len()..].trim();
        // Parse the alpha value (may end with `;` or whitespace).
        let alpha_str = after
            .split(|c: char| c.is_whitespace() || c == ';')
            .next()
            .unwrap_or("");
        if let Ok(alpha) = alpha_str.parse::<f64>() {
            let clean = sql[..pos].trim_end_matches(|c: char| c.is_whitespace() || c == ';');
            return (clean, Some(alpha));
        }
    }
    (sql, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_clause() {
        let sql = "SELECT id FROM docs WHERE embedding <=> :q LIMIT 10 WITH RECALL_GUARANTEE 0.99";
        let (clean, alpha) = strip_recall_clause(sql);
        assert_eq!(alpha, Some(0.99));
        assert!(!clean.contains("RECALL"), "clean={clean}");
    }

    #[test]
    fn passthrough_when_absent() {
        let sql = "SELECT * FROM docs LIMIT 5";
        let (clean, alpha) = strip_recall_clause(sql);
        assert!(alpha.is_none());
        assert_eq!(clean, sql);
    }
}
