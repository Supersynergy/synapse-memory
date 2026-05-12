/// Hash-based TF-IDF embedding: deterministic 768-d sparse unit vector.
///
/// Algorithm:
///   1. Tokenise text into 8-char sliding n-grams (and word tokens).
///   2. Hash each token with xxh3 → index in [0, 768).
///   3. Accumulate weight 1/sqrt(freq+1) per bucket (soft-IDF).
///   4. L2-normalise.
///
/// Properties:
///   - Similar texts → overlapping n-grams → cosine > 0.7.
///   - Unrelated texts → sparse overlap → cosine < 0.3.
///   - Pure Rust, no ML deps. Suitable for in-process use.
pub fn embed_thesis_v2(text: &str) -> Vec<f32> {
    use xxhash_rust::xxh3::xxh3_64;

    const DIM: usize = 768;
    let mut v = vec![0.0f32; DIM];

    let lower = text.to_lowercase();
    let bytes = lower.as_bytes();
    let n = bytes.len();

    if n == 0 {
        return v;
    }

    // Word tokens
    for word in lower.split_whitespace() {
        let h = xxh3_64(word.as_bytes()) as usize % DIM;
        v[h] += 1.0;
    }

    // 4-gram character n-grams
    if n >= 4 {
        for i in 0..=(n - 4) {
            let h = xxh3_64(&bytes[i..i+4]) as usize % DIM;
            v[h] += 0.5;
        }
    }

    // 8-gram character n-grams
    if n >= 8 {
        for i in 0..=(n - 8) {
            let h = xxh3_64(&bytes[i..i+8]) as usize % DIM;
            v[h] += 0.25;
        }
    }

    // L2-normalise
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

/// Legacy stub kept for back-compat. Prefer `embed_thesis_v2`.
pub fn embed_thesis(text: &str) -> Vec<f32> {
    embed_thesis_v2(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        dot / (na * nb + 1e-9)
    }

    #[test]
    fn similar_texts_high_cosine() {
        let a = embed_thesis_v2("bitcoin price rises sharply today");
        let b = embed_thesis_v2("bitcoin price increases sharply today");
        let c = cosine(&a, &b);
        assert!(c > 0.7, "similar texts cosine={c:.3} expected >0.7");
    }

    #[test]
    fn unrelated_texts_low_cosine() {
        let a = embed_thesis_v2("bitcoin price rises sharply today");
        let b = embed_thesis_v2("the weather in munich is cold and rainy");
        let c = cosine(&a, &b);
        assert!(c < 0.3, "unrelated texts cosine={c:.3} expected <0.3");
    }

    #[test]
    fn deterministic() {
        let a = embed_thesis_v2("same text here");
        let b = embed_thesis_v2("same text here");
        assert_eq!(a, b);
    }

    #[test]
    fn unit_norm() {
        let v = embed_thesis_v2("norm check");
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "norm={norm}");
    }
}
