//! Confidence calibration via Platt scaling (linear lookup table).
use anyhow::Result;

pub const N_BUCKETS: usize = 10;

/// Map raw score (0..1) to a bucket index.
pub fn score_to_bucket(score: f64) -> i64 {
    ((score * N_BUCKETS as f64).floor() as i64).clamp(0, N_BUCKETS as i64 - 1)
}

/// Apply calibration correction to a raw score.
pub fn calibrate(store: &crate::LearnStore, raw_score: f64) -> Result<f64> {
    let bucket = score_to_bucket(raw_score);
    let correction = store.get_calibration(bucket)?;
    Ok((raw_score * correction).clamp(0.0, 1.0))
}

/// Rebuild score calibration from explicit context-pack outcomes.
///
/// Scores are normalized when the pack is logged. Passed packs contribute the
/// documents actually used; failed packs contribute all candidates as misses.
/// Beta smoothing and a tight clamp keep small samples from destabilizing rank.
pub fn update_calibration(store: &crate::LearnStore) -> Result<usize> {
    let mut stmt = store.conn.prepare(
        "SELECT doc_ids, score_json, used_doc_ids, accepted_doc_id, reward
         FROM context_query_log WHERE reward IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<i64>>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;

    let mut wins = [0usize; N_BUCKETS];
    let mut samples = [0usize; N_BUCKETS];
    for row in rows {
        let (doc_json, score_json, used_json, accepted, reward) = row?;
        let doc_ids: Vec<i64> = serde_json::from_str(&doc_json).unwrap_or_default();
        let scores: Vec<f64> = serde_json::from_str(&score_json).unwrap_or_default();
        let mut used: std::collections::BTreeSet<i64> =
            serde_json::from_str::<Vec<i64>>(&used_json)
                .unwrap_or_default()
                .into_iter()
                .collect();
        if let Some(id) = accepted {
            used.insert(id);
        }
        for (doc_id, score) in doc_ids.iter().zip(scores.iter()) {
            let positive = reward == 1 && used.contains(doc_id);
            if reward == 1 && !positive {
                continue;
            }
            let bucket = score_to_bucket(score.clamp(0.0, 1.0)) as usize;
            samples[bucket] += 1;
            if positive {
                wins[bucket] += 1;
            }
        }
    }
    let total_samples: usize = samples.iter().sum();
    if total_samples == 0 {
        return Ok(0);
    }
    let total_wins: usize = wins.iter().sum();
    let global_rate = (total_wins as f64 + 2.0) / (total_samples as f64 + 4.0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let tx = store.conn.unchecked_transaction()?;
    let mut updated = 0usize;
    for bucket in 0..N_BUCKETS {
        let correction = if samples[bucket] == 0 {
            1.0
        } else {
            updated += 1;
            let bucket_rate = (wins[bucket] as f64 + 1.0) / (samples[bucket] as f64 + 2.0);
            (bucket_rate / global_rate).clamp(0.75, 1.25)
        };
        tx.execute(
            "INSERT INTO learn_calibration(bucket, correction, samples, updated_ts)
             VALUES(?1,?2,?3,?4)
             ON CONFLICT(bucket) DO UPDATE SET
                 correction=excluded.correction,
                 samples=excluded.samples,
                 updated_ts=excluded.updated_ts",
            rusqlite::params![bucket as i64, correction, samples[bucket] as i64, now],
        )?;
    }
    tx.commit()?;
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_boundaries() {
        assert_eq!(score_to_bucket(0.0), 0);
        assert_eq!(score_to_bucket(0.99), 9);
        assert_eq!(score_to_bucket(0.55), 5);
    }

    #[test]
    fn calibration_learns_from_context_outcomes() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let store = crate::LearnStore::open(tmp.path()).unwrap();
        let kinds = vec!["decision".to_string()];
        store
            .log_context_query(&crate::ContextQueryLog {
                context_id: "good",
                ts: 1,
                query: "q",
                mode: "auto",
                route: "lexical",
                doc_ids: &[1],
                scores: &[0.8],
                kinds: &kinds,
                budget_chars: 100,
                used_chars: 50,
            })
            .unwrap();
        store.reward_context("good", Some(1), &[1], true).unwrap();
        store
            .log_context_query(&crate::ContextQueryLog {
                context_id: "bad",
                ts: 2,
                query: "q",
                mode: "auto",
                route: "lexical",
                doc_ids: &[2],
                scores: &[0.2],
                kinds: &kinds,
                budget_chars: 100,
                used_chars: 50,
            })
            .unwrap();
        store.reward_context("bad", None, &[], false).unwrap();

        assert_eq!(update_calibration(&store).unwrap(), 2);
        assert!(store.get_calibration(8).unwrap() > 1.0);
        assert!(store.get_calibration(2).unwrap() < 1.0);
    }
}
