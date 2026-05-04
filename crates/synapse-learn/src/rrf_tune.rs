//! Adaptive hybrid-RRF-alpha via Thompson sampling per query-shape.
use anyhow::Result;
use rand::RngExt;
use statrs::distribution::{Beta, ContinuousCDF};

pub const ALPHA_BUCKETS: [f64; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

/// Hash a query to u8 shape bucket: first-token-len + has-digit + has-quote.
pub fn query_shape_hash(query: &str) -> u8 {
    let first_len = query
        .split_whitespace()
        .next()
        .map(|t| t.len())
        .unwrap_or(0);
    let has_digit = query.chars().any(|c| c.is_ascii_digit()) as u8;
    let has_quote = query.contains('"') as u8;
    ((first_len & 0x3F) as u8)
        .wrapping_add(has_digit << 6)
        .wrapping_add(has_quote << 7)
}

pub fn pick_alpha(store: &crate::LearnStore, shape_hash: u8) -> Result<(usize, f64)> {
    let mut rng = rand::rng();
    let mut best_bucket = 2usize;
    let mut best_sample = -1f64;

    for bucket in 0..ALPHA_BUCKETS.len() {
        // Each bucket has independent (w, l) — we need to store them separately.
        // For simplicity we use the shape_hash + bucket combined key via the same table.
        // Use a compound key: shape_hash * 16 + bucket (fits in u16→i64)
        let key = (shape_hash as i64) * 16 + bucket as i64;
        let (w, l) = {
            let r = store.conn.query_row(
                "SELECT wins, losses FROM learn_rrf_alpha WHERE shape_hash=?1",
                rusqlite::params![key],
                |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u32>(1)?)),
            );
            match r {
                Ok(v) => v,
                Err(rusqlite::Error::QueryReturnedNoRows) => (1u32, 1u32),
                Err(e) => return Err(e.into()),
            }
        };
        let u: f64 = rng.random_range(0.0..1.0);
        let sample = Beta::new(w as f64, l as f64)
            .map(|b| b.inverse_cdf(u.clamp(1e-9, 1.0 - 1e-9)))
            .unwrap_or(0.5);
        if sample > best_sample {
            best_sample = sample;
            best_bucket = bucket;
        }
    }
    Ok((best_bucket, ALPHA_BUCKETS[best_bucket]))
}

pub fn reward_alpha(
    store: &crate::LearnStore,
    shape_hash: u8,
    bucket: usize,
    hit: bool,
) -> Result<()> {
    let key = (shape_hash as i64) * 16 + bucket as i64;
    if hit {
        store.conn.execute(
            "INSERT INTO learn_rrf_alpha(shape_hash,bucket,wins,losses) VALUES(?1,?2,2,1)
             ON CONFLICT(shape_hash) DO UPDATE SET wins=wins+1, bucket=excluded.bucket",
            rusqlite::params![key, bucket as i64],
        )?;
    } else {
        store.conn.execute(
            "INSERT INTO learn_rrf_alpha(shape_hash,bucket,wins,losses) VALUES(?1,?2,1,2)
             ON CONFLICT(shape_hash) DO UPDATE SET losses=losses+1",
            rusqlite::params![key, bucket as i64],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_hash_deterministic() {
        assert_eq!(
            query_shape_hash("hello world"),
            query_shape_hash("hello world")
        );
        assert_ne!(
            query_shape_hash("hello 123"),
            query_shape_hash("hello world")
        );
    }
}
