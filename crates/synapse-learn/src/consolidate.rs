//! Near-dup merge via text SimHash + band-blocking.
//!
//! Reads `docs` (a real table — cheap PK-ordered scans) instead of `docs_vec`:
//! vec0 materializes each embedding blob per row (~30-100ms/row on large
//! brains), which made the old LSH-over-embeddings pass take 10+ minutes and
//! gigabytes of RAM. Text SimHash catches the actual junk this tool exists for
//! — repeated telepathy/session dumps — in seconds.
use anyhow::Result;
use synapse_pack::{hamming, simhash};

/// Hamming distance ≤ this counts as a near-dup (same bar as pack-time dedup).
pub const HAMMING_THRESHOLD: u32 = 3;
/// 4 bands × 16 bits — sharing one band makes two docs candidates.
const N_BANDS: u8 = 4;
const BAND_BITS: u8 = 16;

#[derive(Debug)]
pub struct MergeReport {
    pub pairs_found: usize,
    pub merged: usize,
    /// True when the comparison budget cut the sweep short — re-run later to
    /// continue converging instead of one unbounded pass.
    pub truncated: bool,
    /// Wall time for the scan+simhash+band+pair pass.
    pub scan_ms: u64,
    /// Wall time for the merge transaction.
    pub merge_ms: u64,
}

/// Hard cap on pairwise comparisons per run — bounds wall time no matter how
/// hot the dup cluster is.
const PAIR_BUDGET: usize = 5_000_000;

/// Bounded near-dup merge: scans at most `max_docs` docs (id-ordered) starting
/// at `offset`. `usize::MAX` = full sweep for scheduled deep maintenance.
pub fn run_consolidate(
    conn: &rusqlite::Connection,
    max_docs: usize,
    offset: usize,
) -> Result<MergeReport> {
    let scan_start = std::time::Instant::now();
    let mut stmt = conn.prepare("SELECT id, text FROM docs ORDER BY id LIMIT ? OFFSET ?")?;
    let rows: Vec<(i64, String)> = stmt
        .query_map(
            rusqlite::params![
                max_docs.min(i64::MAX as usize) as i64,
                offset.min(i64::MAX as usize) as i64
            ],
            |r| Ok((r.get(0)?, r.get::<_, String>(1)?)),
        )?
        .filter_map(|r| r.ok())
        .collect();

    if rows.len() < 2 {
        return Ok(MergeReport {
            pairs_found: 0,
            merged: 0,
            truncated: false,
            scan_ms: scan_start.elapsed().as_millis() as u64,
            merge_ms: 0,
        });
    }

    let hashes: Vec<u64> = rows.iter().map(|(_, text)| simhash(text)).collect();

    // Band-blocking: candidate pairs share at least one 16-bit band.
    let mut bands: std::collections::HashMap<u64, Vec<usize>> = std::collections::HashMap::new();
    for (i, h) in hashes.iter().enumerate() {
        for band in 0..N_BANDS {
            let band_val = (h >> (band * BAND_BITS)) & 0xFFFF;
            let key = ((band as u64) << 48) | band_val;
            bands.entry(key).or_default().push(i);
        }
    }

    let mut pairs_found = 0;
    let mut comparisons = 0usize;
    let mut truncated = false;
    // dup_id → keep_id (smaller id wins — earliest copy survives)
    let mut merges: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    'bands: for idxs in bands.values() {
        for i in 0..idxs.len() {
            for j in (i + 1)..idxs.len() {
                comparisons += 1;
                if comparisons > PAIR_BUDGET {
                    truncated = true;
                    break 'bands;
                }
                let (ai, bi) = (idxs[i], idxs[j]);
                if hamming(hashes[ai], hashes[bi]) <= HAMMING_THRESHOLD {
                    pairs_found += 1;
                    let (keep, dup) = if rows[ai].0 < rows[bi].0 {
                        (ai, bi)
                    } else {
                        (bi, ai)
                    };
                    merges.insert(rows[dup].0, rows[keep].0);
                }
            }
        }
    }

    let scan_ms = scan_start.elapsed().as_millis() as u64;
    let merge_start = std::time::Instant::now();
    // One transaction — autocommitted UPDATEs cost a WAL fsync each.
    conn.execute_batch("BEGIN")?;
    let mut merged = 0usize;
    for (dup_id, keep_id) in &merges {
        if conn
            .execute(
                "UPDATE docs SET meta = json_patch(COALESCE(meta,'{}'), json_object('merged_into', ?1)) WHERE id=?2",
                rusqlite::params![keep_id, dup_id],
            )
            .is_ok()
        {
            merged += 1;
        }
    }
    conn.execute_batch("COMMIT")?;

    Ok(MergeReport {
        pairs_found,
        merged,
        truncated,
        scan_ms,
        merge_ms: merge_start.elapsed().as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simhash_dedup_marks_identical_texts() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE docs(id INTEGER PRIMARY KEY, text TEXT, meta TEXT)")
            .unwrap();
        conn.execute(
            "INSERT INTO docs(text) VALUES ('the quick brown fox jumps over')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO docs(text) VALUES ('the quick brown fox jumps over')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO docs(text) VALUES ('completely different document here')",
            [],
        )
        .unwrap();
        let rep = run_consolidate(&conn, 100, 0).unwrap();
        assert_eq!(rep.merged, 1, "the two identical texts must merge");
        let meta: String = conn
            .query_row("SELECT meta FROM docs WHERE id=2", [], |r| r.get(0))
            .unwrap();
        assert!(meta.contains("merged_into"), "dup doc gets merge marker");
    }
}
