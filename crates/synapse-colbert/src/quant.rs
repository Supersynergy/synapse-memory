//! int8 symmetric quantization for ColBERT token vectors.
use synapse_kernel::dot_i8;

pub type QuantizedTokenVec = (Vec<i8>, f32);

/// Quantize f32 slice → (i8 values, scale).
/// scale = max(|v|) / 127.0
/// q[i] = clamp(round(v[i] / scale), -127, 127)
pub fn quant_i8(vec: &[f32]) -> QuantizedTokenVec {
    let max_abs = vec.iter().map(|v| v.abs()).fold(0f32, f32::max);
    if max_abs == 0.0 {
        return (vec![0i8; vec.len()], 1.0);
    }
    let scale = max_abs / 127.0;
    let q = vec
        .iter()
        .map(|&v| (v / scale).round().clamp(-127.0, 127.0) as i8)
        .collect();
    (q, scale)
}

/// Dequantize i8 + scale → f32 (for debug / accuracy checks).
pub fn dequant_i8(q: &[i8], scale: f32) -> Vec<f32> {
    q.iter().map(|&v| v as f32 * scale).collect()
}

/// ColBERT max-sim over i8 quantized token vectors.
/// query_vecs: Vec<(i8 tokens, scale)>, doc_vecs: same.
/// Returns f32 score (dequantized).
pub fn max_sim_i8(query_vecs: &[QuantizedTokenVec], doc_vecs: &[QuantizedTokenVec]) -> f32 {
    if query_vecs.is_empty() || doc_vecs.is_empty() {
        return 0.0;
    }
    query_vecs
        .iter()
        .map(|(qv, qs)| {
            doc_vecs
                .iter()
                .map(|(dv, ds)| {
                    let raw = dot_i8(qv, dv);
                    raw as f32 * qs * ds
                })
                .fold(f32::NEG_INFINITY, f32::max)
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_accuracy() {
        let v: Vec<f32> = (0..128).map(|i| (i as f32 - 64.0) / 64.0).collect();
        let (q, scale) = quant_i8(&v);
        let v2 = dequant_i8(&q, scale);
        let max_err = v
            .iter()
            .zip(&v2)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        // max error should be < 1 LSB = scale
        assert!(max_err <= scale + 1e-6, "max_err={max_err} scale={scale}");
    }

    #[test]
    fn zero_vec() {
        let v = vec![0f32; 16];
        let (q, scale) = quant_i8(&v);
        assert!(q.iter().all(|&x| x == 0));
        assert_eq!(scale, 1.0);
    }

    #[test]
    fn dot_i8_correctness() {
        let a = vec![1i8, 2, 3, 4];
        let b = vec![4i8, 3, 2, 1];
        assert_eq!(dot_i8(&a, &b), 20);
    }

    #[test]
    fn max_sim_i8_vs_f32() {
        // unit vectors: i8 quant of [1,0,0,0] and [0,1,0,0]
        let v1: Vec<f32> = vec![1.0, 0.0, 0.0, 0.0];
        let v2: Vec<f32> = vec![0.0, 1.0, 0.0, 0.0];
        let (q1, s1) = quant_i8(&v1);
        let (q2, s2) = quant_i8(&v2);
        // query=[v1], doc=[v1,v2] → max_sim should be ~1.0 (q1·q1 > q1·q2)
        let score = max_sim_i8(&[(q1.clone(), s1)], &[(q1, s1), (q2, s2)]);
        assert!((score - 1.0).abs() < 0.02, "score={score}");
    }

    #[test]
    fn storage_ratio() {
        // 10 token vecs of dim 128: f32=4 bytes, i8=1 byte → 4× smaller
        let dim = 128usize;
        let n_tokens = 10usize;
        let f32_bytes = n_tokens * dim * 4;
        let i8_bytes = n_tokens * dim * 1 + n_tokens * 4; // i8 data + f32 scales
        assert!(f32_bytes / i8_bytes >= 3, "ratio={}", f32_bytes / i8_bytes);
    }
}
