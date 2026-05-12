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

    /// Execute vec-search. Returns Err until embedding pipeline is wired.
    /// Gap: synapsql has no embedding pipeline at the SQL wire layer; callers
    /// must resolve `:param` to `&[f32]` before calling this.
    pub fn execute(&self, _query_vec: &[f32]) -> Result<Vec<(u64, f32)>, VecSearchError> {
        Err(VecSearchError::EmbeddingPipelineNotWired)
    }
}

/// Error returned when vec-search cannot proceed.
#[derive(Debug)]
pub enum VecSearchError {
    /// Embedding pipeline not available at the SQL wire layer.
    /// Wire `Store::search_vec` + an embed model to resolve.
    EmbeddingPipelineNotWired,
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

    #[test]
    fn execute_returns_error_not_empty_vec() {
        let sql = "SELECT id FROM docs WHERE embedding <=> :q LIMIT 5";
        let op = VectorOp::parse(sql).unwrap();
        let result = op.execute(&[0.1_f32, 0.2, 0.3]);
        assert!(result.is_err(), "execute must return Err (not silent vec![]) until embedding pipeline is wired");
        matches!(result.unwrap_err(), VecSearchError::EmbeddingPipelineNotWired);
    }
}
