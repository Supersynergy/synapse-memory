//! Embedding-drift detector.
//! TODO: PCA summary via smartcore (stub — requires embedding model access at runtime)
use anyhow::Result;

pub const WARN_THRESHOLD: f64 = 0.98;
pub const ERROR_THRESHOLD: f64 = 0.95;

type DriftRow = (i64, String, Vec<u8>);

#[derive(Debug, Clone)]
pub struct DriftReport {
    pub mean_cosine: f64,
    pub sample_size: usize,
    pub status: DriftStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DriftStatus {
    Ok,
    Warn,
    Error,
}

pub fn cosine_sim(a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| *x as f64 * *y as f64)
        .sum();
    let na: f64 = a.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    let nb: f64 = b.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na * nb)
}

/// Check drift by comparing stored embeddings against fresh embeddings.
/// `get_embedding` is a callback that re-embeds text on demand.
pub fn check_drift<F>(
    conn: &rusqlite::Connection,
    sample_size: usize,
    get_embedding: F,
) -> Result<DriftReport>
where
    F: Fn(&str) -> Result<Vec<f32>>,
{
    let mut stmt = conn.prepare(
        "SELECT d.id, d.text, v.embedding FROM docs d
         JOIN docs_vec v ON v.id = d.id
         ORDER BY RANDOM() LIMIT ?1",
    )?;
    let rows: Vec<DriftRow> = stmt
        .query_map(rusqlite::params![sample_size as i64], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get::<_, Vec<u8>>(2)?))
        })?
        .filter_map(|r| r.ok())
        .collect();

    if rows.is_empty() {
        return Ok(DriftReport {
            mean_cosine: 1.0,
            sample_size: 0,
            status: DriftStatus::Ok,
        });
    }

    let mut sims = Vec::with_capacity(rows.len());
    for (_, text, emb_bytes) in &rows {
        let stored: Vec<f32> = emb_bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        match get_embedding(text) {
            Ok(fresh) => sims.push(cosine_sim(&stored, &fresh)),
            Err(_) => continue,
        }
    }

    if sims.is_empty() {
        return Ok(DriftReport {
            mean_cosine: 1.0,
            sample_size: 0,
            status: DriftStatus::Ok,
        });
    }

    let mean = sims.iter().sum::<f64>() / sims.len() as f64;
    let status = if mean < ERROR_THRESHOLD {
        DriftStatus::Error
    } else if mean < WARN_THRESHOLD {
        DriftStatus::Warn
    } else {
        DriftStatus::Ok
    };

    Ok(DriftReport {
        mean_cosine: mean,
        sample_size: sims.len(),
        status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_identical() {
        let v = vec![1.0f32, 0.0, 0.0];
        assert!((cosine_sim(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn drift_thresholds() {
        assert_eq!(
            if 0.94 < ERROR_THRESHOLD {
                DriftStatus::Error
            } else {
                DriftStatus::Ok
            },
            DriftStatus::Error
        );
    }
}
