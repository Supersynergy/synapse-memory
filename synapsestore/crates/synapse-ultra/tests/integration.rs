use ndarray::Array2;
use synapse_ultra::binary::{hamming_distance, pack_signs};
use synapse_ultra::cache::{CacheKey, T0Cache};
use synapse_ultra::index::Hit;
use synapse_ultra::search::top_k_f32;

#[test]
fn test_binary_pack_signs_all_positive() {
    let v = vec![1.0f32; 384];
    let packed = pack_signs(&v);
    assert_eq!(packed.len(), 48);
    assert!(packed.iter().all(|&b| b == 0xFF));
}

#[test]
fn test_hamming_self_zero() {
    let v = vec![0.3f32; 384];
    let p = pack_signs(&v);
    assert_eq!(hamming_distance(&p, &p), 0);
}

#[test]
fn test_hamming_opposite_is_max() {
    let pos = pack_signs(&vec![1.0f32; 384]);
    let neg = pack_signs(&vec![-1.0f32; 384]);
    assert_eq!(hamming_distance(&pos, &neg), 384);
}

#[test]
fn test_top_k_f32_ordering() {
    let dim = 384usize;
    let mut q = vec![0.0f32; dim];
    q[0] = 1.0;

    let mut row0 = vec![0.0f32; dim]; row0[0] = 1.0; // identical
    let mut row1 = vec![0.0f32; dim]; row1[1] = 1.0; // orthogonal

    let data: Vec<f32> = row0.iter().chain(row1.iter()).cloned().collect();
    let matrix = Array2::from_shape_vec((2, dim), data).unwrap();
    let results = top_k_f32(&q, matrix.view(), 2);
    assert_eq!(results[0].0, 0);
    assert!((results[0].1 - 1.0).abs() < 1e-5);
}

#[test]
fn test_cache_roundtrip() {
    let cache = T0Cache::new(256);
    let key = CacheKey::new("test query", 1, 10);
    let hits = vec![Hit { id: 42, score: 0.99 }];
    cache.put(key, hits);
    let got = cache.get(&key).unwrap();
    assert_eq!(got[0].id, 42);
}

#[test]
fn test_cache_invalidate() {
    let cache = T0Cache::new(256);
    let key = CacheKey::new("test", 1, 5);
    cache.put(key, vec![Hit { id: 1, score: 0.5 }]);
    cache.invalidate();
    assert!(cache.get(&key).is_none());
}

#[test]
fn test_binary_first_recall() {
    // Build 100 random vectors, ensure binary_first recalls ≥80% of strict top-10
    let n = 1000usize;
    let dim = 384usize;
    // Use pseudo-random data (not structured sine waves) for meaningful recall test
    let data: Vec<f32> = (0..n * dim).map(|i| {
        let x = (i as f64 * 1.6180339887) % 1.0;
        (x * 2.0 - 1.0) as f32
    }).collect();
    let mut matrix_f32 = Array2::from_shape_vec((n, dim), data).unwrap();
    synapse_ultra::snapshot::normalize_rows(&mut matrix_f32);
    let matrix_f16: Vec<u16> = matrix_f32.iter()
        .map(|&x| half::f16::from_f32(x).to_bits())
        .collect();
    let bin_matrix = synapse_ultra::binary::build_binary_matrix(&matrix_f32);

    let mut q = vec![0.0f32; dim];
    q[0] = 1.0; q[7] = 0.5;
    let norm = q.iter().map(|x| x * x).sum::<f32>().sqrt();
    q.iter_mut().for_each(|x| *x /= norm);
    let q_sign = pack_signs(&q);

    let strict = top_k_f32(&q, matrix_f32.view(), 10);
    // rerank_n=400 gives ≥95% recall on random data per theory (400/1000=40%)
    let binary = synapse_ultra::search::top_k_binary_first(
        &q, &q_sign, &bin_matrix, &matrix_f16, n, 10, 400
    );

    let strict_ids: std::collections::HashSet<usize> = strict.iter().map(|x| x.0).collect();
    let overlap = binary.iter().filter(|x| strict_ids.contains(&x.0)).count();
    let recall = overlap as f32 / 10.0;
    println!("recall@10 binary_first vs strict: {:.2}", recall);
    assert!(recall >= 0.80, "recall too low: {}", recall);
}
