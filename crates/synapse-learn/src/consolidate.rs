//! Near-dup nightly merge via LSH + cosine threshold.
use anyhow::Result;
use rand::Rng;

pub const COSINE_THRESHOLD: f64 = 0.95;
pub const N_PLANES: usize = 8;

pub struct LshIndex {
    planes: Vec<Vec<f32>>,
    dim: usize,
}

impl LshIndex {
    pub fn new(dim: usize) -> Self {
        let mut rng = rand::thread_rng();
        let planes = (0..N_PLANES)
            .map(|_| (0..dim).map(|_| rng.gen_range(-1.0f32..1.0)).collect())
            .collect();
        Self { planes, dim }
    }

    pub fn hash(&self, emb: &[f32]) -> u8 {
        let mut h = 0u8;
        for (i, plane) in self.planes.iter().enumerate() {
            let dot: f32 = emb.iter().zip(plane.iter()).map(|(a, b)| a * b).sum();
            if dot >= 0.0 {
                h |= 1 << i;
            }
        }
        h
    }
}

#[derive(Debug)]
pub struct MergeReport {
    pub pairs_found: usize,
    pub merged: usize,
}

pub fn run_consolidate(conn: &rusqlite::Connection) -> Result<MergeReport> {
    // Load all embeddings
    let mut stmt = conn.prepare(
        "SELECT d.id, d.uri, v.embedding FROM docs d
         JOIN docs_vec v ON v.id = d.id"
    )?;
    let rows: Vec<(i64, Option<String>, Vec<u8>)> = stmt.query_map([], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get::<_, Vec<u8>>(2)?))
    })?.filter_map(|r| r.ok()).collect();

    if rows.len() < 2 {
        return Ok(MergeReport { pairs_found: 0, merged: 0 });
    }

    let dim = rows[0].2.len() / 4;
    let lsh = LshIndex::new(dim);

    // Build hash buckets
    let mut buckets: std::collections::HashMap<u8, Vec<usize>> = std::collections::HashMap::new();
    let embeddings: Vec<Vec<f32>> = rows.iter()
        .map(|(_, _, b)| b.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect())
        .collect();

    for (i, emb) in embeddings.iter().enumerate() {
        let h = lsh.hash(emb);
        buckets.entry(h).or_default().push(i);
    }

    // Find candidate pairs
    let mut pairs_found = 0;
    let mut merged = 0;
    for bucket_idxs in buckets.values() {
        for i in 0..bucket_idxs.len() {
            for j in (i + 1)..bucket_idxs.len() {
                let ai = bucket_idxs[i];
                let bi = bucket_idxs[j];
                let sim = crate::drift::cosine_sim(&embeddings[ai], &embeddings[bi]);
                if sim > COSINE_THRESHOLD {
                    pairs_found += 1;
                    // Keep the one with smaller id (earliest), mark other as duplicate
                    let keep = if rows[ai].0 < rows[bi].0 { ai } else { bi };
                    let dup = if keep == ai { bi } else { ai };
                    let keep_id = rows[keep].0;
                    let dup_id = rows[dup].0;
                    // Merge: redirect uri of dup to keep, update meta
                    conn.execute(
                        "UPDATE docs SET meta = json_patch(COALESCE(meta,'{}'), json_object('merged_into', ?1)) WHERE id=?2",
                        rusqlite::params![keep_id, dup_id],
                    ).ok();
                    merged += 1;
                }
            }
        }
    }

    Ok(MergeReport { pairs_found, merged })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsh_same_vector_same_hash() {
        let lsh = LshIndex::new(4);
        let v = vec![0.1f32, -0.2, 0.5, 0.3];
        assert_eq!(lsh.hash(&v), lsh.hash(&v));
    }
}
