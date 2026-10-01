use crate::error::Result;
use rusqlite::functions::FunctionFlags;

const DIM: usize = 384;

/// Derive a deterministic unit-length f32 vector from arbitrary text using
/// BLAKE3. Each of the 384 dimensions is drawn from successive 4-byte
/// windows of the hash output (XOF mode), then the vector is L2-normalised.
fn hash_vec(text: &str) -> [f32; DIM] {
    let mut out = [0u8; DIM * 4];
    let mut xof = blake3::Hasher::new();
    xof.update(text.as_bytes());
    xof.finalize_xof().fill(&mut out);

    let mut v = [0f32; DIM];
    for i in 0..DIM {
        let bytes = [out[i * 4], out[i * 4 + 1], out[i * 4 + 2], out[i * 4 + 3]];
        v[i] = i32::from_le_bytes(bytes) as f32;
    }

    // L2 normalise
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
    v
}

fn cosine(a: &[f32; DIM], b: &[f32; DIM]) -> f32 {
    // Both vectors are already unit-length, so dot product == cosine similarity.
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum::<f32>()
}

/// Register `synapse_match(text TEXT, query TEXT) -> REAL` on `conn`.
///
/// Returns a cosine similarity in [0.0, 1.0] based on BLAKE3-derived hash
/// vectors. Phase 5 Day 57+: swap `hash_vec` for fastembed inference.
pub fn register_synapse_match(conn: &rusqlite::Connection) -> Result<()> {
    conn.create_scalar_function(
        "synapse_match",
        2,
        FunctionFlags::SQLITE_DETERMINISTIC | FunctionFlags::SQLITE_UTF8,
        |ctx| {
            let text: String = ctx.get(0)?;
            let query: String = ctx.get(1)?;
            let va = hash_vec(&text);
            let vb = hash_vec(&query);
            // Cosine of unit vecs is in [-1, 1]; clamp to [0, 1] for SQL callers.
            let score = ((cosine(&va, &vb) + 1.0) / 2.0).clamp(0.0, 1.0) as f64;
            Ok(score)
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synapse_match_smoke() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        register_synapse_match(&conn).unwrap();
        let s: f64 = conn
            .query_row("SELECT synapse_match('hello world', 'world')", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(s >= 0.0 && s <= 1.0, "score out of range: {s}");
    }

    #[test]
    fn synapse_match_identical() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        register_synapse_match(&conn).unwrap();
        let s: f64 = conn
            .query_row(
                "SELECT synapse_match('rust web framework', 'rust web framework')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        // Identical strings → cosine == 1.0 → scaled to 1.0
        assert!((s - 1.0).abs() < 1e-6, "identical should be 1.0, got {s}");
    }
}
