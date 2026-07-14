//! Query-accept feedback loop.
use anyhow::Result;

pub fn record_accept(
    store: &crate::LearnStore,
    query: &str,
    accepted_doc_id: i64,
    shard_id: &str,
) -> Result<()> {
    record_context_outcome(
        store,
        query,
        Some(accepted_doc_id),
        &[accepted_doc_id],
        true,
        shard_id,
    )
}

/// Close the retrieval loop for a generated context pack.
///
/// `passed=false` is an explicit miss, not an inferred absence. Positive packs
/// may name the accepted document and/or every document actually used.
pub fn record_context_outcome(
    store: &crate::LearnStore,
    query: &str,
    accepted_doc_id: Option<i64>,
    used_doc_ids: &[i64],
    passed: bool,
    shard_id: &str,
) -> Result<()> {
    if let Some(context_id) = query.strip_prefix("context:") {
        store.reward_context(context_id, accepted_doc_id, used_doc_ids, passed)?;
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    if let Some(accepted_doc_id) = accepted_doc_id {
        store.log_feedback(ts, query, None, accepted_doc_id)?;
    }
    store.update_bandit(shard_id, passed)?;
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
            |r| r.get::<_, i64>(0).map(|v| v as usize),
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

    #[test]
    fn context_failure_is_recorded_explicitly() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let store = LearnStore::open(tmp.path()).unwrap();
        store
            .log_context_query(&crate::ContextQueryLog {
                context_id: "abc",
                ts: 1,
                query: "query",
                mode: "auto",
                route: "lexical",
                doc_ids: &[7, 8],
                scores: &[0.8, 0.2],
                kinds: &["decision".into(), "note".into()],
                budget_chars: 100,
                used_chars: 80,
            })
            .unwrap();
        record_context_outcome(&store, "context:abc", None, &[], false, "default").unwrap();
        let reward: i64 = store
            .conn
            .query_row(
                "SELECT reward FROM context_query_log WHERE context_id='abc'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(reward, 0);
    }

    #[test]
    fn context_feedback_rejects_unseen_doc_ids() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let store = LearnStore::open(tmp.path()).unwrap();
        store
            .log_context_query(&crate::ContextQueryLog {
                context_id: "abc",
                ts: 1,
                query: "query",
                mode: "auto",
                route: "lexical",
                doc_ids: &[7],
                scores: &[0.8],
                kinds: &["decision".into()],
                budget_chars: 100,
                used_chars: 80,
            })
            .unwrap();
        assert!(
            record_context_outcome(&store, "context:abc", Some(99), &[99], true, "default")
                .is_err()
        );
    }
}
