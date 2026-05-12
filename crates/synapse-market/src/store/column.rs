/// Column encoders/decoders — delta-encode timestamps, plain f32 for OHLCV.
/// No SIMD yet (W3 task). Branchless where easy.

/// Delta-encode a slice of i64 timestamps → (base, deltas: Vec<i32>).
/// Assumes sorted input.
pub fn delta_encode_ts(ts: &[i64]) -> (i64, Vec<i32>) {
    if ts.is_empty() {
        return (0, vec![]);
    }
    let base = ts[0];
    let deltas = ts.iter().map(|&t| (t - base) as i32).collect();
    (base, deltas)
}

pub fn delta_decode_ts(base: i64, deltas: &[i32]) -> Vec<i64> {
    deltas.iter().map(|&d| base + d as i64).collect()
}

/// Dict-encode f32 column when cardinality is low (e.g., ≤256 distinct values).
/// Returns None if not worth it (>256 distinct or n < 16).
pub fn dict_encode_f32(values: &[f32]) -> Option<(Vec<f32>, Vec<u8>)> {
    if values.len() < 16 {
        return None;
    }
    let mut dict: Vec<f32> = Vec::new();
    let mut codes: Vec<u8> = Vec::with_capacity(values.len());
    for &v in values {
        let pos = dict.iter().position(|&d| (d - v).abs() < 1e-7);
        match pos {
            Some(i) => codes.push(i as u8),
            None => {
                if dict.len() >= 256 {
                    return None; // too many distinct
                }
                codes.push(dict.len() as u8);
                dict.push(v);
            }
        }
    }
    // Only worth it if dict is small relative to data
    let raw_bytes = values.len() * 4;
    let encoded_bytes = dict.len() * 4 + values.len(); // dict f32s + u8 codes
    if encoded_bytes < raw_bytes {
        Some((dict, codes))
    } else {
        None
    }
}

pub fn dict_decode_f32(dict: &[f32], codes: &[u8]) -> Vec<f32> {
    codes.iter().map(|&c| dict[c as usize]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_roundtrip() {
        let ts: Vec<i64> = (0..100).map(|i| 1_700_000_000 + i * 900).collect();
        let (base, deltas) = delta_encode_ts(&ts);
        let decoded = delta_decode_ts(base, &deltas);
        assert_eq!(ts, decoded);
    }

    #[test]
    fn dict_encode_constant() {
        let vals = vec![42.0f32; 64];
        let result = dict_encode_f32(&vals);
        assert!(result.is_some());
        let (dict, codes) = result.unwrap();
        assert_eq!(dict.len(), 1);
        let decoded = dict_decode_f32(&dict, &codes);
        assert_eq!(decoded, vals);
    }

    #[test]
    fn dict_encode_high_cardinality_skips() {
        let vals: Vec<f32> = (0..300).map(|i| i as f32).collect();
        assert!(dict_encode_f32(&vals).is_none());
    }
}
