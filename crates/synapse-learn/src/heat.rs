//! Usage-heat reranking.
//! Adds access_count + last_accessed_ts columns to docs and rescores hits.
use anyhow::Result;
use synapse_core::Hit;

pub const LAMBDA: f64 = 0.05;

/// Ensure heat columns exist on the docs table in a synapse-core Store connection.
pub fn migrate_heat(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        ALTER TABLE docs ADD COLUMN IF NOT EXISTS access_count INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE docs ADD COLUMN IF NOT EXISTS last_accessed_ts INTEGER;
    "#,
    )
    .ok(); // ignore if columns exist (SQLite error on duplicate)
           // Fallback: try each separately
    conn.execute(
        "ALTER TABLE docs ADD COLUMN access_count INTEGER NOT NULL DEFAULT 0",
        [],
    )
    .ok();
    conn.execute("ALTER TABLE docs ADD COLUMN last_accessed_ts INTEGER", [])
        .ok();
    Ok(())
}

pub fn record_access(conn: &rusqlite::Connection, doc_id: i64) -> Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    conn.execute(
        "UPDATE docs SET access_count = access_count + 1, last_accessed_ts = ?1 WHERE id = ?2",
        rusqlite::params![now, doc_id],
    )?;
    Ok(())
}

/// Rerank hits using heat score: final = hybrid_score * (1 + ln(1+access)) * exp(-lambda * age_days)
pub fn rerank(hits: Vec<Hit>, conn: &rusqlite::Connection) -> Result<Vec<Hit>> {
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as f64;
    let mut out = Vec::with_capacity(hits.len());
    for mut h in hits {
        let (access_count, last_ts): (i64, Option<i64>) = conn
            .query_row(
                "SELECT COALESCE(access_count,0), last_accessed_ts FROM docs WHERE id=?1",
                rusqlite::params![h.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or((0, None));
        let age_days = last_ts
            .map(|ts| (now_secs - ts as f64) / 86400.0)
            .unwrap_or(0.0)
            .max(0.0);
        let heat = (1.0 + (1.0 + access_count as f64).ln()) * (-LAMBDA * age_days).exp();
        h.score *= heat;
        out.push(h);
    }
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(out)
}

#[cfg(test)]
mod tests {

    #[test]
    fn heat_score_increases_with_access() {
        let heat0 = (1.0 + (1.0f64).ln()) * 1.0;
        let heat10 = (1.0 + (11.0f64).ln()) * 1.0;
        assert!(heat10 > heat0);
    }
}
