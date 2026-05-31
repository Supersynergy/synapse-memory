//! Per-day regime embedding + brute-force cosine similarity search.
//!
//! Feature vec (5-dim f32):
//!   [ret_1d, ret_5d, vol_20d_normalized, range_norm, volume_ratio]
//!
//! Storage: `regime_<symbol>(ts INTEGER PRIMARY KEY, vec BLOB NOT NULL)`
//! Brute-force dot-product scan → <1ms @ 10k days (NEON auto-vectorized).

use crate::error::{Error, Result};
use crate::{OhlcvRow, RegimeHit};
use rusqlite::{Connection, params};

const FEAT_DIM: usize = 5;

fn table_name(symbol: &str) -> String {
    let safe: String = symbol
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("regime_{}", safe.to_uppercase())
}

fn ensure_table(conn: &Connection, symbol: &str) -> Result<()> {
    let t = table_name(symbol);
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS {t} (
            ts  INTEGER PRIMARY KEY,
            vec BLOB NOT NULL
        ) WITHOUT ROWID;"
    ))?;
    Ok(())
}

/// Encode f32 slice to bytes.
fn encode(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Decode bytes to f32 vec.
fn decode(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

/// Compute feature vec for a given timestamp using preceding rows.
/// Needs at least 20 rows of context; returns None if insufficient data.
fn compute_features(rows: &[OhlcvRow], idx: usize) -> Option<[f32; FEAT_DIM]> {
    if idx == 0 {
        return None;
    }
    let (_, _, h, l, c, vol) = rows[idx];
    let (_, _, _, _, c_prev, vol_prev) = rows[idx - 1];

    let ret_1d = if c_prev != 0.0 {
        ((c / c_prev) - 1.0) as f32
    } else {
        0.0
    };

    let ret_5d = if idx >= 5 {
        let (_, _, _, _, c5, _) = rows[idx - 5];
        if c5 != 0.0 {
            ((c / c5) - 1.0) as f32
        } else {
            0.0
        }
    } else {
        0.0
    };

    let window = &rows[idx.saturating_sub(20)..=idx];
    let returns: Vec<f64> = window
        .windows(2)
        .map(|w| {
            let c0 = w[0].5;
            let c1 = w[1].5;
            if c0 != 0.0 { c1 / c0 - 1.0 } else { 0.0 }
        })
        .collect();
    let mean = returns.iter().sum::<f64>() / returns.len().max(1) as f64;
    let vol_20d = (returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>()
        / returns.len().max(1) as f64)
        .sqrt() as f32;

    let range_norm = if c != 0.0 { ((h - l) / c) as f32 } else { 0.0 };
    let vol_ratio = if vol_prev > 0.0 {
        (vol / vol_prev) as f32
    } else {
        1.0
    };

    Some([ret_1d, ret_5d, vol_20d, range_norm, vol_ratio])
}

fn normalize(v: &mut [f32; FEAT_DIM]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-9 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Build regime embeddings for all rows of `symbol` and store them.
pub fn build_all(conn: &Connection, symbol: &str, rows: &[OhlcvRow]) -> Result<()> {
    ensure_table(conn, symbol)?;
    let t = table_name(symbol);
    conn.execute_batch("BEGIN;")?;
    {
        let sql = format!("INSERT OR REPLACE INTO {t} (ts, vec) VALUES (?1, ?2)");
        let mut stmt = conn.prepare_cached(&sql)?;
        for (idx, &(ts, _, _, _, _, _)) in rows.iter().enumerate() {
            if let Some(mut feat) = compute_features(rows, idx) {
                normalize(&mut feat);
                stmt.execute(params![ts, encode(&feat)])?;
            }
        }
    }
    conn.execute_batch("COMMIT;")?;
    Ok(())
}

/// Cosine similarity (dot-product on L2-normalized vecs).
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Find top-n similar past days to `date_ts` for `symbol`.
pub fn search(
    conn: &Connection,
    symbol: &str,
    date_ts: i64,
    top_n: usize,
) -> Result<Vec<RegimeHit>> {
    let t = table_name(symbol);
    // Check table exists
    let exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            params![t],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;
    if !exists {
        return Err(Error::Market(format!("no regime data for {symbol}")));
    }

    // Load query vec
    let query_blob: Vec<u8> = conn
        .query_row(
            &format!("SELECT vec FROM {t} WHERE ts = ?1"),
            params![date_ts],
            |r| r.get(0),
        )
        .map_err(|_| Error::Market(format!("no regime vec for ts={date_ts}")))?;
    let query = decode(&query_blob);

    // Brute-force scan all past rows (ts < date_ts)
    let mut stmt = conn.prepare_cached(&format!("SELECT ts, vec FROM {t} WHERE ts < ?1"))?;
    let mut scores: Vec<RegimeHit> = stmt
        .query_map(params![date_ts], |r| {
            let ts: i64 = r.get(0)?;
            let blob: Vec<u8> = r.get(1)?;
            Ok((ts, blob))
        })?
        .filter_map(|r| r.ok())
        .map(|(ts, blob)| {
            let v = decode(&blob);
            (ts, cosine(&query, &v))
        })
        .collect();

    scores.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scores.truncate(top_n);
    Ok(scores)
}

pub use build_all as RegimeVec;
