//! `<=>` cosine-distance operator parser + executor.
//!
//! Rewrites `SELECT ... WHERE col <=> :q LIMIT k` into a
//! synapse-core vec-search call, returns ranked rows.
//!
//! # Syntax
//! ```sql
//! SELECT id, body FROM docs WHERE embedding <=> :query_vec LIMIT 10;
//! ```
//!
//! # Wire
//! Detected by `contains("<=>")`; parsed by `VectorOp::parse`.

/// A parsed `<=>` expression extracted from a SQL string.
#[derive(Debug, Clone)]
pub struct VectorOp {
    /// Column holding the stored embedding.
    pub column: String,
    /// Bind-param name for the query vector (`:param`).
    pub param: String,
    /// LIMIT k from the original query.
    pub k: usize,
}

impl VectorOp {
    /// Returns `Some(VectorOp)` if `sql` contains a `<=>` distance expression.
    /// Minimal parser: `<column> <=> :<param>`.
    pub fn parse(sql: &str) -> Option<Self> {
        // Fast-path: skip if no operator.
        let arrow_pos = sql.find("<=>")?;
        let before = sql[..arrow_pos].trim_end();
        let column = before.rsplit_once(' ')
            .map(|(_, c)| c)
            .unwrap_or(before)
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '_')
            .to_owned();

        let after = sql[arrow_pos + 3..].trim_start();
        let param = after
            .split(|c: char| c.is_whitespace() || c == ',' || c == ')')
            .next()
            .unwrap_or("")
            .trim_start_matches(':')
            .to_owned();

        // Extract LIMIT k (best-effort).
        let k = sql.to_ascii_uppercase()
            .find("LIMIT")
            .and_then(|p| sql[p + 5..].trim().split_whitespace().next()
                .and_then(|n| n.parse().ok()))
            .unwrap_or(10);

        Some(VectorOp { column, param, k })
    }

    /// Placeholder executor — calls into synapse-core vec-search.
    /// TODO: wire real Store::vec_search once API is stable.
    pub fn execute(&self, _query_vec: &[f32]) -> Vec<(u64, f32)> {
        // TODO: store.vec_search(self.column, query_vec, self.k)
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic() {
        let sql = "SELECT id FROM docs WHERE embedding <=> :q LIMIT 5";
        let op = VectorOp::parse(sql).unwrap();
        assert_eq!(op.column, "embedding");
        assert_eq!(op.param, "q");
        assert_eq!(op.k, 5);
    }

    #[test]
    fn no_op_returns_none() {
        assert!(VectorOp::parse("SELECT * FROM docs").is_none());
    }
}
