//! Query-accept feedback loop.
use anyhow::Result;

pub fn record_accept(
    store: &crate::LearnStore,
    query: &str,
    accepted_doc_id: i64,
    shard_id: &str,
) -> Result<()> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    store.log_feedback(ts, query, None, accepted_doc_id)?;
    store.update_bandit(shard_id, true)?;
    Ok(())
}

/// Sweep: mark any feedback older than `timeout_secs` with no accept as a miss.
/// In practice the CLI logs accepts; non-accepts are inferred by absence.
/// This sweeper runs per `synapse feedback --sweep`.
pub fn sweep_unaccepted(
    store: &crate::LearnStore,
    shard_id: &str,
    timeout_secs: i64,
) -> Result<usize> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let cutoff = now - timeout_secs;
    let count: usize = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM feedback WHERE ts < ?1",
            rusqlite::params![cutoff],
            |r| r.get::<_, usize>(0),
        )
        .unwrap_or(0);
    if count > 0 {
        store.update_bandit(shard_id, false)?;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LearnStore;

    #[test]
    fn feedback_record() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let store = LearnStore::open(tmp.path()).unwrap();
        record_accept(&store, "test query", 42, "shard0").unwrap();
        let (w, _) = store.get_bandit_prior("shard0").unwrap();
        assert!(w >= 1);
    }
}
