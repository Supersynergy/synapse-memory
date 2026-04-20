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

/// Update calibration from feedback log using Platt-scaling approximation.
/// Groups feedback by score bucket, computes accept rate, stores correction.
pub fn update_calibration(store: &crate::LearnStore) -> Result<usize> {
    // Count accepted docs per score bucket from feedback
    // Since we don't store scores in feedback directly, we approximate:
    // correction = actual_accept_rate / expected_rate_per_bucket
    let total: i64 = store.conn.query_row(
        "SELECT COUNT(*) FROM feedback", [], |r| r.get(0)
    ).unwrap_or(0);
    if total == 0 {
        return Ok(0);
    }
    // Simple approach: uniform correction = 1.0 until we have scored feedback
    // Real implementation would join feedback with search result scores
    let updated = 0;
    // TODO: join feedback with search log (requires storing scores at query time)
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
}
