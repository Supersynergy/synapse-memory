//! `HYBRID_RANK(text_col, vec_col, query)` SQL UDF.
//!
//! Fuses BM25/FTS5 score + cosine-distance via RRF (Reciprocal Rank Fusion).
//!
//! # Syntax
//! ```sql
//! SELECT id, HYBRID_RANK(body, embedding, :q) AS score
//! FROM docs
//! ORDER BY score DESC
//! LIMIT 20;
//! ```
//!
//! # Registration (rusqlite)
//! Call `register_udf(conn)` after opening a Connection.

/// RRF constant — 60 is standard.
const K: f64 = 60.0;

/// Fuse BM25 rank + vector rank via RRF.
pub fn hybrid_rank(bm25_rank: u64, vec_rank: u64) -> f64 {
    1.0 / (K + bm25_rank as f64) + 1.0 / (K + vec_rank as f64)
}

/// Register `HYBRID_RANK` as a scalar UDF on an open rusqlite connection.
/// Signature: `HYBRID_RANK(bm25_rank INTEGER, vec_rank INTEGER) → REAL`
///
/// NOTE: Full 3-arg `HYBRID_RANK(text, embedding, query)` requires FTS5 +
/// vec-search sub-queries; that wiring is TODO — this exposes the RRF math now.
#[cfg(feature = "rusqlite-udf")]
pub fn register_udf(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.create_scalar_function(
        "HYBRID_RANK",
        2,
        rusqlite::functions::FunctionFlags::SQLITE_UTF8
            | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let bm25: i64 = ctx.get(0)?;
            let vec:  i64 = ctx.get(1)?;
            Ok(hybrid_rank(bm25.unsigned_abs(), vec.unsigned_abs()))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrf_smoke() {
        let score = hybrid_rank(1, 1);
        assert!(score > 0.0 && score < 1.0, "score={score}");
    }

    #[test]
    fn lower_rank_higher_score() {
        assert!(hybrid_rank(1, 1) > hybrid_rank(10, 10));
    }
}
