/// Deterministic stub: returns a pseudo-random 768-d unit vector seeded by hash of `text`.
/// Replace with MLX-phi4 call in W6.
pub fn embed_thesis(text: &str) -> Vec<f32> {
    let seed = blake3::hash(text.as_bytes());
    let seed_bytes = seed.as_bytes();

    let mut v: Vec<f32> = (0..768)
        .map(|i| {
            let b0 = seed_bytes[i % 32] as u64;
            let b1 = seed_bytes[(i + 1) % 32] as u64;
            let raw = ((b0 << 8 | b1) ^ (i as u64 * 6364136223846793005)) as f32;
            // Map to [-1, 1]
            (raw % 65536.0) / 32768.0 - 1.0
        })
        .collect();

    // L2-normalise
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    v.iter_mut().for_each(|x| *x /= norm);
    v
}
