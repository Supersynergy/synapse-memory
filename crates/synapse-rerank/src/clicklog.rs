use anyhow::Result;
use rusqlite::Connection;

pub struct Features {
    pub vec_score: f32,
    pub fts_score: f32,
    pub recency_days: f32,
    pub title_exact_match: bool,
    pub path_depth: i32,
    pub doc_len_log: f32,
}

pub fn ensure_schema(db: &Connection) -> Result<()> {
    db.execute_batch(include_str!(
        "../../../python/migrations/add_rerank_log.sql"
    ))?;
    Ok(())
}

pub fn log_rerank_event(
    db: &Connection,
    query_id: &str,
    doc_id: &str,
    features: &Features,
    clicked: bool,
) -> Result<()> {
    db.execute(
        "INSERT INTO synapse_rerank_log \
         (query_id, doc_id, vec_score, fts_score, recency_days, title_exact_match, path_depth, doc_len, clicked) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            query_id,
            doc_id,
            features.vec_score,
            features.fts_score,
            features.recency_days,
            features.title_exact_match as i32,
            features.path_depth,
            features.doc_len_log as i32,
            clicked as i32,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicklog_roundtrip() {
        let db = Connection::open_in_memory().unwrap();
        ensure_schema(&db).unwrap();

        let f = Features {
            vec_score: 0.9,
            fts_score: 0.8,
            recency_days: 7.0,
            title_exact_match: true,
            path_depth: 2,
            doc_len_log: 5.3,
        };
        log_rerank_event(&db, "q1", "doc42", &f, true).unwrap();

        let (query_id, clicked): (String, i32) = db
            .query_row(
                "SELECT query_id, clicked FROM synapse_rerank_log WHERE doc_id = 'doc42'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(query_id, "q1");
        assert_eq!(clicked, 1);
    }
}
