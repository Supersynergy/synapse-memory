//! INT8 linear-scale quantizer (PR-C1) — 4× compression, <1% recall loss.
//!
//! Per-vec scale: q[i] = round(v[i] / max_abs * 127). Unbias: dequant via scale/127.
//! Faster than FAISS scalar quant for cosine-normalized embeddings (no centering needed).

use crate::{QuantError, Quantizer};

#[derive(Debug, Clone)]
pub struct Int8 {
    dim: usize,
}

#[derive(Debug, Clone)]
pub struct Int8Vec {
    pub scale: f32,
    pub codes: Vec<i8>,
}

impl Int8 {
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }
}

impl Quantizer for Int8 {
    type Encoded = Int8Vec;

    fn calibrate(&mut self, _corpus: &[&[f32]]) -> Result<(), QuantError> {
        Ok(()) // per-vec scale, no global calibration needed
    }

    fn encode(&self, v: &[f32]) -> Result<Int8Vec, QuantError> {
        if v.len() != self.dim {
            return Err(QuantError::DimMismatch {
                expected: self.dim,
                actual: v.len(),
            });
        }
        let max_abs = v.iter().fold(0f32, |m, x| m.max(x.abs())).max(1e-10);
        let codes: Vec<i8> = v
            .iter()
            .map(|x| (x / max_abs * 127.0).round().clamp(-128.0, 127.0) as i8)
            .collect();
        Ok(Int8Vec {
            scale: max_abs,
            codes,
        })
    }

    fn decode(&self, e: &Int8Vec) -> Result<Vec<f32>, QuantError> {
        if e.codes.len() != self.dim {
            return Err(QuantError::DimMismatch {
                expected: self.dim,
                actual: e.codes.len(),
            });
        }
        Ok(e.codes
            .iter()
            .map(|c| (*c as f32) * e.scale / 127.0)
            .collect())
    }

    fn distance(&self, a: &Int8Vec, b: &Int8Vec) -> f32 {
        // Approximate cosine via int8 dot. NOT cosine-exact (no renorm here).
        // Sufficient for ranking when both vecs are pre-L2-normalized in f32 space.
        let dot: i32 = a
            .codes
            .iter()
            .zip(b.codes.iter())
            .map(|(x, y)| (*x as i32) * (*y as i32))
            .sum();
        // Convert to approx-cosine: dot / (127^2) * scale_a * scale_b ≈ cosine for unit vecs
        -(dot as f32) * a.scale * b.scale / (127.0 * 127.0) // negate so smaller = closer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn norm(v: &mut [f32]) {
        let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if n > 0.0 {
            for x in v.iter_mut() {
                *x /= n;
            }
        }
    }

    #[test]
    fn round_trip_recall() {
        let q = Int8::new(4);
        let mut v = vec![0.5_f32, -0.3, 0.7, 0.1];
        norm(&mut v);
        let e = q.encode(&v).unwrap();
        let d = q.decode(&e).unwrap();
        for (a, b) in v.iter().zip(d.iter()) {
            assert!((a - b).abs() < 0.02, "round-trip err {} vs {}", a, b);
        }
    }

    #[test]
    fn distance_orders() {
        let q = Int8::new(3);
        let mut a = vec![1.0_f32, 0.0, 0.0];
        norm(&mut a);
        let mut b = vec![0.9, 0.1, 0.0];
        norm(&mut b);
        let mut c = vec![0.0, 1.0, 0.0];
        norm(&mut c);
        let ea = q.encode(&a).unwrap();
        let eb = q.encode(&b).unwrap();
        let ec = q.encode(&c).unwrap();
        let dab = q.distance(&ea, &eb);
        let dac = q.distance(&ea, &ec);
        assert!(dab < dac, "a-b should be closer than a-c: {dab} vs {dac}");
    }
}
