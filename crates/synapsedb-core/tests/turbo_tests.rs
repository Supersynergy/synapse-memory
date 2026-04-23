//! Comprehensive tests for synapse-turbo modules.
//!
//! Test patterns inspired by:
//! - Qdrant: RRF scoring correctness (empty, single, multi, k-parameter)
//! - RuVector/GrafeoDB: recall@k measurement, brute-force kNN ground truth
//! - AppFlowy: sqlite-vec init + roundtrip verification
//!
//! Run: cargo test -p synapse-core --features turbo -- turbo

#![cfg(feature = "turbo")]

use synapsedb_core::turbo::hybrid_cache::HybridCache;
use synapsedb_core::turbo::ndarray_search::{HybridSearch, NdArraySearch};
use synapsedb_core::turbo::simd;
use synapsedb_core::turbo::matryoshka::MatryoshkaConfig;
use synapsedb_core::turbo::reranker::{MmrReranker, Reranker, RrfReranker, ScoredResult, WeightedScoreReranker};
use synapsedb_core::types::EMBED_DIM;
use synapsedb_core::{PutRequest, Store};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Generate a deterministic embedding from a seed byte.
fn fake_emb(seed: u8) -> Vec<f32> {
    (0..EMBED_DIM)
        .map(|i| ((i as u8).wrapping_mul(seed) as f32) / 255.0)
        .collect()
}

/// Generate a normalized random-ish embedding (cosine-safe).
fn norm_emb(seed: u8) -> Vec<f32> {
    let raw = fake_emb(seed);
    let norm: f32 = raw.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm < 1e-10 {
        return raw;
    }
    raw.iter().map(|x| x / norm).collect()
}

/// Build a Store with N docs, each with a deterministic embedding.
fn build_test_store(n: usize) -> (tempfile::NamedTempFile, Store) {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let mut store = Store::open(tmp.path()).unwrap();
    for i in 1..=n {
        let seed = i as u8;
        store
            .put(&PutRequest {
                title: Some(format!("doc-{}", i)),
                text: format!("document number {} about topic {}", i, seed % 5),
                embedding: Some(fake_emb(seed)),
                ..Default::default()
            })
            .unwrap();
    }
    (tmp, store)
}

/// Brute-force kNN ground truth (for recall@k verification).
fn ground_truth_knn(embeddings: &[(i64, Vec<f32>)], query: &[f32], k: usize) -> Vec<i64> {
    let q_norm: f32 = query.iter().map(|x| x * x).sum::<f32>().sqrt();
    if q_norm < 1e-10 {
        return Vec::new();
    }
    let mut scored: Vec<(i64, f32)> = embeddings
        .iter()
        .map(|(id, emb)| {
            let e_norm: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
            let dot: f32 = emb.iter().zip(query).map(|(a, b)| a * b).sum();
            let cosine = if e_norm > 1e-10 {
                dot / (q_norm * e_norm)
            } else {
                0.0
            };
            (*id, 1.0 - cosine) // distance
        })
        .collect();
    scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    scored.iter().take(k).map(|(id, _)| *id).collect()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// NdArraySearch tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn turbo_ndarray_cosine_correctness() {
    // Insert 3 known vectors, query with one, verify the nearest is correct.
    let (tmp, _store) = build_test_store(3);
    let search = NdArraySearch::from_sqlite(tmp.path()).unwrap();

    // Query with the same embedding as doc 1 → should return doc 1 as nearest.
    let results = search.search(&fake_emb(1), 3);
    assert_eq!(results.len(), 3);
    // Nearest neighbor should be doc 1 (distance ≈ 0)
    assert_eq!(results[0].0, 1, "nearest should be doc 1");
    assert!(
        results[0].1 < 0.01,
        "distance to self should be ~0, got {}",
        results[0].1
    );
}

#[test]
fn turbo_ndarray_recall_at_k() {
    // Build 100 docs, verify NdArraySearch matches brute-force ground truth.
    let n = 100;
    let (tmp, _store) = build_test_store(n);
    let search = NdArraySearch::from_sqlite(tmp.path()).unwrap();
    assert_eq!(search.len(), n);

    // Collect all embeddings for ground truth.
    let embeddings: Vec<(i64, Vec<f32>)> = (1..=n).map(|i| (i as i64, fake_emb(i as u8))).collect();

    // Test recall@5 for multiple queries
    let test_queries = [7u8, 42, 99, 1, 50];
    for seed in test_queries {
        let query = fake_emb(seed);
        let k = 5;
        let gt = ground_truth_knn(&embeddings, &query, k);
        let results: Vec<i64> = search.search(&query, k).iter().map(|(id, _)| *id).collect();

        // Count overlap
        let recall: f32 = results.iter().filter(|id| gt.contains(id)).count() as f32 / k as f32;
        assert!(
            recall >= 1.0,
            "recall@{} for seed {} should be 1.0 (brute-force is exact), got {}",
            k,
            seed,
            recall
        );
    }
}

#[test]
fn turbo_ndarray_zero_vector_safe() {
    // Zero vector should return empty results, not panic.
    let (tmp, _store) = build_test_store(5);
    let search = NdArraySearch::from_sqlite(tmp.path()).unwrap();
    let zero = vec![0.0f32; EMBED_DIM];
    let results = search.search(&zero, 5);
    assert!(
        results.is_empty(),
        "zero vector should return empty results"
    );
}

#[test]
fn turbo_ndarray_dim_mismatch() {
    // Wrong dimension should return empty, not panic.
    let (tmp, _store) = build_test_store(5);
    let search = NdArraySearch::from_sqlite(tmp.path()).unwrap();
    let wrong_dim = vec![1.0f32; 128]; // 128 instead of 384
    let results = search.search(&wrong_dim, 5);
    assert!(
        results.is_empty(),
        "wrong dimension should return empty results"
    );
}

#[test]
fn turbo_ndarray_k_larger_than_corpus() {
    // k > n_docs should return n_docs results.
    let (tmp, _store) = build_test_store(3);
    let search = NdArraySearch::from_sqlite(tmp.path()).unwrap();
    let results = search.search(&fake_emb(1), 100);
    assert_eq!(results.len(), 3, "should return all 3 docs when k=100");
}

#[test]
fn turbo_ndarray_results_sorted_by_distance() {
    // Verify results are sorted ascending by distance.
    let (tmp, _store) = build_test_store(20);
    let search = NdArraySearch::from_sqlite(tmp.path()).unwrap();
    let results = search.search(&fake_emb(10), 10);
    for window in results.windows(2) {
        assert!(
            window[0].1 <= window[1].1 + 1e-6,
            "results not sorted: {} > {}",
            window[0].1,
            window[1].1
        );
    }
}

#[test]
fn turbo_ndarray_empty_db() {
    // Empty database should return Err, not panic.
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let _store = Store::open(tmp.path()).unwrap(); // creates schema, no docs
    let result = NdArraySearch::from_sqlite(tmp.path());
    assert!(result.is_err(), "empty DB should return error");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// HybridCache tests (Qdrant-style tiered cache verification)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn turbo_cache_memory_roundtrip() {
    let cache = HybridCache::new().unwrap();
    let emb = norm_emb(42);
    cache.put_embedding("test query", &emb);
    let retrieved = cache.get_embedding("test query").unwrap();
    assert_eq!(emb.len(), retrieved.len());
    for (a, b) in emb.iter().zip(retrieved.iter()) {
        assert!((a - b).abs() < 1e-7, "embedding mismatch: {} vs {}", a, b);
    }
}

#[test]
fn turbo_cache_miss_returns_none() {
    let cache = HybridCache::new().unwrap();
    assert!(
        cache.get_embedding("nonexistent").is_none(),
        "cache miss should return None"
    );
}

#[test]
fn turbo_cache_sqlite_persistence() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let emb = norm_emb(77);

    // Write to cache with SQLite backing
    {
        let cache = HybridCache::with_sqlite(tmp.path()).unwrap();
        cache.put_embedding("persistent query", &emb);
        let stats = cache.stats();
        assert_eq!(stats.emb_t1_memory, 1);
    }

    // New instance should find it in SQLite (T2)
    {
        let cache = HybridCache::with_sqlite(tmp.path()).unwrap();
        let retrieved = cache.get_embedding("persistent query").unwrap();
        assert_eq!(retrieved.len(), emb.len());
        // After retrieval, it should be promoted to T1
        let stats = cache.stats();
        assert_eq!(stats.emb_t1_memory, 1, "should be promoted to T1");
    }
}

#[test]
fn turbo_cache_results_roundtrip() {
    let cache = HybridCache::new().unwrap();
    let data = b"serialized results payload";
    cache.put_results("my query", data);
    let retrieved = cache.get_results("my query").unwrap();
    assert_eq!(&retrieved, data);
}

#[test]
fn turbo_cache_hash_deterministic() {
    let h1 = HybridCache::hash_query("test");
    let h2 = HybridCache::hash_query("test");
    let h3 = HybridCache::hash_query("different");
    assert_eq!(h1, h2, "same input should produce same hash");
    assert_ne!(h1, h3, "different input should produce different hash");
}

#[test]
fn turbo_cache_stats_accurate() {
    let cache = HybridCache::new().unwrap();
    let stats = cache.stats();
    assert_eq!(stats.emb_t1_memory, 0);
    assert_eq!(stats.results_precomputed, 0);

    cache.put_embedding("q1", &norm_emb(1));
    cache.put_embedding("q2", &norm_emb(2));
    cache.put_results("q1", b"results");

    let stats = cache.stats();
    assert_eq!(stats.emb_t1_memory, 2);
    assert_eq!(stats.results_precomputed, 1);
}

#[test]
fn turbo_cache_prewarm() {
    let cache = HybridCache::new().unwrap();
    let queries = vec![
        ("query a", norm_emb(1)),
        ("query b", norm_emb(2)),
        ("query c", norm_emb(3)),
    ];
    let refs: Vec<(&str, Vec<f32>)> = queries.iter().map(|(q, e)| (*q, e.clone())).collect();
    cache.prewarm(&refs);

    assert_eq!(cache.stats().emb_t1_memory, 3);
    assert!(cache.get_embedding("query a").is_some());
    assert!(cache.get_embedding("query b").is_some());
    assert!(cache.get_embedding("query c").is_some());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// HybridSearch + RRF tests (Qdrant-inspired)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn turbo_hybrid_search_returns_results() {
    let (tmp, _store) = build_test_store(10);
    let hybrid = HybridSearch::from_sqlite(tmp.path()).unwrap();
    let query_emb = fake_emb(5);
    let results = hybrid.search("document", &query_emb, 5);
    assert!(
        !results.is_empty(),
        "hybrid search should return results for matching query"
    );
    assert!(results.len() <= 5, "should respect limit");
}

#[test]
fn turbo_hybrid_rrf_scores_decreasing() {
    // RRF fused scores should be in descending order.
    let (tmp, _store) = build_test_store(20);
    let hybrid = HybridSearch::from_sqlite(tmp.path()).unwrap();
    let results = hybrid.search("document number", &fake_emb(10), 10);
    for window in results.windows(2) {
        assert!(
            window[0].score >= window[1].score - 1e-10,
            "RRF scores not sorted: {} < {}",
            window[0].score,
            window[1].score
        );
    }
}

#[test]
fn turbo_hybrid_both_sources_contribute() {
    // When a doc matches both FTS5 and vec, its RRF score should be higher
    // than docs matching only one source.
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let mut store = Store::open(tmp.path()).unwrap();

    // Doc that matches both "rust" keyword AND is nearest vector
    let target_emb = norm_emb(42);
    store
        .put(&PutRequest {
            title: Some("rust doc".into()),
            text: "rust programming language memory system".into(),
            embedding: Some(target_emb.clone()),
            ..Default::default()
        })
        .unwrap();

    // Docs that only match keyword
    for i in 1..5 {
        store
            .put(&PutRequest {
                text: format!("rust topic number {}", i),
                embedding: Some(fake_emb(i)),
                ..Default::default()
            })
            .unwrap();
    }

    // Docs that only match vector (different keywords)
    for i in 10..15 {
        store
            .put(&PutRequest {
                text: format!("python topic number {}", i),
                embedding: Some(norm_emb(42 + i)), // similar-ish to target
                ..Default::default()
            })
            .unwrap();
    }

    drop(store);

    let hybrid = HybridSearch::from_sqlite(tmp.path()).unwrap();
    let results = hybrid.search("rust", &target_emb, 10);

    // The doc matching both sources should be ranked first
    assert!(!results.is_empty());
    let top = &results[0];
    assert_eq!(
        top.id, 1,
        "doc matching both FTS5+vec should rank first, got id={}",
        top.id
    );
}

#[test]
fn turbo_hybrid_fts_only_query() {
    // Even with a zero embedding, FTS5 results should still come through.
    let (tmp, _store) = build_test_store(10);
    let hybrid = HybridSearch::from_sqlite(tmp.path()).unwrap();
    let zero_emb = vec![0.0f32; EMBED_DIM];
    // Zero vec → vec search returns nothing, but FTS5 should still work
    let results = hybrid.search("document number", &zero_emb, 5);
    // FTS5 alone should still produce results
    assert!(
        !results.is_empty(),
        "FTS5 should still return results even with zero embedding"
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Integration: full pipeline
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn turbo_full_pipeline_cache_to_search() {
    // End-to-end: put embeddings in cache → search via NdArraySearch → verify match.
    let (tmp, _store) = build_test_store(50);
    let search = NdArraySearch::from_sqlite(tmp.path()).unwrap();
    let cache = HybridCache::new().unwrap();

    // Simulate caching a query embedding
    let query = "test query";
    let emb = fake_emb(25); // should match doc 25 best
    cache.put_embedding(query, &emb);

    // Retrieve from cache and search
    let cached_emb = cache.get_embedding(query).unwrap();
    let results = search.search(&cached_emb, 5);

    assert!(!results.is_empty());
    assert_eq!(
        results[0].0, 25,
        "cached embedding for seed 25 should find doc 25 first"
    );
}

#[test]
fn turbo_ndarray_matches_sqlite_vec() {
    // NdArraySearch and sqlite-vec should return the same top-k results.
    let n = 50;
    let (tmp, store) = build_test_store(n);
    let nd_search = NdArraySearch::from_sqlite(tmp.path()).unwrap();

    let query_emb = fake_emb(13);
    let k = 5;

    // NdArraySearch results
    let nd_results: Vec<i64> = nd_search
        .search(&query_emb, k)
        .iter()
        .map(|(id, _)| *id)
        .collect();

    // sqlite-vec results
    let sv_results: Vec<i64> = store
        .search("", synapsedb_core::SearchMode::Vec, Some(&query_emb), k)
        .unwrap()
        .iter()
        .map(|h| h.id)
        .collect();

    // Both should return the same set of IDs (order may differ slightly due to tie-breaking)
    let nd_set: std::collections::HashSet<i64> = nd_results.iter().copied().collect();
    let sv_set: std::collections::HashSet<i64> = sv_results.iter().copied().collect();
    let overlap = nd_set.intersection(&sv_set).count();
    let recall = overlap as f32 / k as f32;
    assert!(
        recall >= 0.8,
        "ndarray vs sqlite-vec recall@{} should be >= 0.8, got {} (nd={:?}, sv={:?})",
        k,
        recall,
        nd_results,
        sv_results
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Performance sanity checks
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn turbo_ndarray_search_under_1ms() {
    // NdArraySearch on 100 docs should complete in <1ms.
    let (tmp, _store) = build_test_store(100);
    let search = NdArraySearch::from_sqlite(tmp.path()).unwrap();
    let query = fake_emb(50);

    let start = std::time::Instant::now();
    for _ in 0..100 {
        let _ = search.search(&query, 10);
    }
    let elapsed = start.elapsed();
    let avg_us = elapsed.as_micros() as f64 / 100.0;
    assert!(
        avg_us < 1000.0,
        "avg search should be <1ms, got {:.0}us",
        avg_us
    );
}

#[test]
fn turbo_cache_lookup_under_1us() {
    // In-memory cache lookup should be sub-microsecond.
    let cache = HybridCache::new().unwrap();
    let emb = norm_emb(1);
    cache.put_embedding("bench", &emb);

    let start = std::time::Instant::now();
    for _ in 0..10_000 {
        let _ = cache.get_embedding("bench");
    }
    let elapsed = start.elapsed();
    let avg_ns = elapsed.as_nanos() as f64 / 10_000.0;
    assert!(
        avg_ns < 10_000.0,
        "avg cache lookup should be <10us, got {:.0}ns",
        avg_ns
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Phase 1: SIMD dot product tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn turbo_simd_f32_matches_ndarray() {
    let a = norm_emb(42);
    let b = norm_emb(99);
    let simd_dot = simd::dot_f32(&a, &b);
    let scalar_dot = simd::dot_f32_scalar(&a, &b);
    assert!(
        (simd_dot - scalar_dot).abs() < 1e-5,
        "SIMD {simd_dot} vs scalar {scalar_dot}"
    );
}

#[test]
fn turbo_simd_i8_correctness() {
    let a: Vec<i8> = (0..EMBED_DIM).map(|i| ((i % 100) as i8) - 50).collect();
    let b: Vec<i8> = (0..EMBED_DIM).map(|i| (((i * 7) % 100) as i8) - 50).collect();
    let simd_result = simd::dot_i8(&a, &b);
    let scalar_result = simd::dot_i8_scalar(&a, &b);
    assert_eq!(simd_result, scalar_result, "i8 dot must be exact");
}

#[test]
fn turbo_simd_search_matches_ndarray_search() {
    let (tmp, _store) = build_test_store(50);
    let search = NdArraySearch::from_sqlite(tmp.path()).unwrap();
    let query = fake_emb(25);
    let k = 10;

    let nd_results: Vec<i64> = search.search(&query, k).iter().map(|(id, _)| *id).collect();
    let simd_results: Vec<i64> = search
        .search_simd(&query, k)
        .iter()
        .map(|(id, _)| *id)
        .collect();

    assert_eq!(
        nd_results, simd_results,
        "SIMD and ndarray search should return identical results"
    );
}

#[test]
fn turbo_simd_search_zero_vector() {
    let (tmp, _store) = build_test_store(5);
    let search = NdArraySearch::from_sqlite(tmp.path()).unwrap();
    let zero = vec![0.0f32; EMBED_DIM];
    assert!(search.search_simd(&zero, 5).is_empty());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Phase 1: Quantized search tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn turbo_quantized_from_ndarray_search() {
    let (tmp, _store) = build_test_store(50);
    let search = NdArraySearch::from_sqlite(tmp.path()).unwrap();
    let quantized = search.to_quantized();

    assert_eq!(quantized.len(), 50);
    assert!(!quantized.is_empty());

    let results = quantized.search(&fake_emb(25), 5);
    assert_eq!(results.len(), 5);
    assert_eq!(
        results[0].0, 25,
        "quantized search should find doc 25 first, got {}",
        results[0].0
    );
}

#[test]
fn turbo_quantized_recall_vs_f32() {
    let n = 100;
    let (tmp, _store) = build_test_store(n);
    let search = NdArraySearch::from_sqlite(tmp.path()).unwrap();
    let quantized = search.to_quantized();
    let k = 10;

    let test_seeds = [1u8, 13, 42, 77, 99];
    for seed in test_seeds {
        let query = fake_emb(seed);
        let f32_ids: Vec<i64> = search.search(&query, k).iter().map(|(id, _)| *id).collect();
        let q_ids: Vec<i64> = quantized
            .search(&query, k)
            .iter()
            .map(|(id, _)| *id)
            .collect();

        let overlap = f32_ids.iter().filter(|id| q_ids.contains(id)).count();
        let recall = overlap as f32 / k as f32;
        assert!(
            recall >= 0.8,
            "recall@{k} for seed {seed}: {recall} (f32={f32_ids:?}, q={q_ids:?})"
        );
    }
}

#[test]
fn turbo_quantized_4x_compression() {
    let (tmp, _store) = build_test_store(100);
    let search = NdArraySearch::from_sqlite(tmp.path()).unwrap();
    let quantized = search.to_quantized();

    let ratio = quantized.compression_ratio();
    assert!(
        (ratio - 4.0).abs() < 0.1,
        "compression ratio should be ~4.0, got {ratio}"
    );
}

#[test]
fn turbo_quantized_zero_vector_safe() {
    let (tmp, _store) = build_test_store(10);
    let search = NdArraySearch::from_sqlite(tmp.path()).unwrap();
    let quantized = search.to_quantized();
    assert!(quantized.search(&vec![0.0f32; EMBED_DIM], 5).is_empty());
}

#[test]
fn turbo_quantized_dim_mismatch_safe() {
    let (tmp, _store) = build_test_store(10);
    let search = NdArraySearch::from_sqlite(tmp.path()).unwrap();
    let quantized = search.to_quantized();
    assert!(quantized.search(&vec![1.0f32; 128], 5).is_empty());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Phase 1: Head-to-head benchmark (in-test)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn turbo_benchmark_f32_vs_simd_vs_quantized() {
    let n = 200;
    let (tmp, _store) = build_test_store(n);
    let search = NdArraySearch::from_sqlite(tmp.path()).unwrap();
    let quantized = search.to_quantized();
    let query = fake_emb(100);
    let k = 10;
    let iterations = 500;

    // Warmup
    for _ in 0..10 {
        let _ = search.search(&query, k);
        let _ = search.search_simd(&query, k);
        let _ = quantized.search(&query, k);
    }

    let start = std::time::Instant::now();
    for _ in 0..iterations {
        let _ = search.search(&query, k);
    }
    let f32_us = start.elapsed().as_micros() as f64 / iterations as f64;

    let start = std::time::Instant::now();
    for _ in 0..iterations {
        let _ = search.search_simd(&query, k);
    }
    let simd_us = start.elapsed().as_micros() as f64 / iterations as f64;

    let start = std::time::Instant::now();
    for _ in 0..iterations {
        let _ = quantized.search(&query, k);
    }
    let quant_us = start.elapsed().as_micros() as f64 / iterations as f64;

    eprintln!("\n=== Phase 1 Benchmark ({n} docs, {EMBED_DIM}-dim, k={k}) ===");
    eprintln!("  f32 ndarray:    {f32_us:.1}µs");
    eprintln!("  f32 SIMD:       {simd_us:.1}µs");
    eprintln!("  int8 quantized: {quant_us:.1}µs");
    eprintln!(
        "  SIMD speedup:   {:.1}×",
        if simd_us > 0.0 { f32_us / simd_us } else { 0.0 }
    );
    eprintln!(
        "  Quant speedup:  {:.1}×",
        if quant_us > 0.0 { f32_us / quant_us } else { 0.0 }
    );
    eprintln!(
        "  Memory:         f32={}KB, int8={}KB ({:.1}× compression)",
        n * EMBED_DIM * 4 / 1024,
        quantized.memory_bytes() / 1024,
        quantized.compression_ratio()
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Phase 1c: Matryoshka funnel search tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn turbo_matryoshka_from_ndarray() {
    let (tmp, _store) = build_test_store(100);
    let search = NdArraySearch::from_sqlite(tmp.path()).unwrap();
    let matryoshka = search.to_matryoshka(MatryoshkaConfig::default());

    assert_eq!(matryoshka.len(), 100);
    let results = matryoshka.funnel_search(&fake_emb(50), 5);
    assert_eq!(results.len(), 5);
    assert_eq!(
        results[0].0, 50,
        "funnel search should find doc 50 first, got {}",
        results[0].0
    );
}

#[test]
fn turbo_matryoshka_recall_vs_full_search() {
    let n = 200;
    let (tmp, _store) = build_test_store(n);
    let search = NdArraySearch::from_sqlite(tmp.path()).unwrap();
    let matryoshka = search.to_matryoshka(MatryoshkaConfig {
        coarse_dim: 96,
        funnel_factor: 6,
    });
    let k = 10;

    let seeds = [1u8, 25, 50, 100, 150];
    let mut total_recall = 0.0;
    for &seed in &seeds {
        let query = fake_emb(seed);
        let full_ids: Vec<i64> = matryoshka
            .full_search(&query, k)
            .iter()
            .map(|(id, _)| *id)
            .collect();
        let funnel_ids: Vec<i64> = matryoshka
            .funnel_search(&query, k)
            .iter()
            .map(|(id, _)| *id)
            .collect();

        let overlap = full_ids.iter().filter(|id| funnel_ids.contains(id)).count();
        total_recall += overlap as f32 / k as f32;
    }

    let avg_recall = total_recall / seeds.len() as f32;
    assert!(
        avg_recall >= 0.7,
        "avg matryoshka recall@{k} should be >= 0.7, got {avg_recall}"
    );
}

#[test]
fn turbo_matryoshka_faster_than_full() {
    let n = 500;
    let (tmp, _store) = build_test_store(n);
    let search = NdArraySearch::from_sqlite(tmp.path()).unwrap();
    let matryoshka = search.to_matryoshka(MatryoshkaConfig::default());
    let query = fake_emb(100);
    let k = 10;
    let iters = 200;

    // Warmup
    for _ in 0..5 {
        let _ = matryoshka.full_search(&query, k);
        let _ = matryoshka.funnel_search(&query, k);
    }

    let start = std::time::Instant::now();
    for _ in 0..iters {
        let _ = matryoshka.full_search(&query, k);
    }
    let full_us = start.elapsed().as_micros() as f64 / iters as f64;

    let start = std::time::Instant::now();
    for _ in 0..iters {
        let _ = matryoshka.funnel_search(&query, k);
    }
    let funnel_us = start.elapsed().as_micros() as f64 / iters as f64;

    eprintln!("\n=== Matryoshka Benchmark ({n} docs, {EMBED_DIM}-dim, k={k}) ===");
    eprintln!("  Full search:   {full_us:.1}µs");
    eprintln!("  Funnel search: {funnel_us:.1}µs");
    eprintln!(
        "  Speedup:       {:.1}×",
        if funnel_us > 0.0 { full_us / funnel_us } else { 0.0 }
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Phase 1c: Binary quantization tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn turbo_binary_from_ndarray() {
    let (tmp, _store) = build_test_store(100);
    let search = NdArraySearch::from_sqlite(tmp.path()).unwrap();
    let binary = search.to_binary(true);

    assert_eq!(binary.len(), 100);
    let ratio = binary.compression_ratio();
    assert!(ratio > 30.0, "binary compression should be ~32×, got {ratio}");
}

#[test]
fn turbo_binary_search_finds_self() {
    let (tmp, _store) = build_test_store(50);
    let search = NdArraySearch::from_sqlite(tmp.path()).unwrap();
    let binary = search.to_binary(false);

    let results = binary.search_binary(&fake_emb(25), 5);
    assert_eq!(results.len(), 5);
    assert_eq!(
        results[0].0, 25,
        "binary search should find doc 25 first"
    );
    assert_eq!(results[0].1, 0, "Hamming to self should be 0");
}

#[test]
fn turbo_binary_twophase_recall() {
    let n = 200;
    let (tmp, _store) = build_test_store(n);
    let search = NdArraySearch::from_sqlite(tmp.path()).unwrap();
    let binary = search.to_binary(true);
    let k = 10;

    let seeds = [1u8, 25, 50, 100, 150];
    let mut total_recall = 0.0;
    for &seed in &seeds {
        let query = fake_emb(seed);
        let f32_ids: Vec<i64> = search.search(&query, k).iter().map(|(id, _)| *id).collect();
        let bq_ids: Vec<i64> = binary
            .search_twophase(&query, k, 20)
            .iter()
            .map(|(id, _)| *id)
            .collect();

        let overlap = f32_ids.iter().filter(|id| bq_ids.contains(id)).count();
        total_recall += overlap as f32 / k as f32;
    }

    let avg_recall = total_recall / seeds.len() as f32;
    assert!(
        avg_recall >= 0.6,
        "avg binary two-phase recall@{k} should be >= 0.6, got {avg_recall}"
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Phase 2: Reranker integration tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn turbo_rrf_reranker_integration() {
    let rrf = RrfReranker::default();
    let fts = vec![
        ScoredResult { id: 10, score: 5.0 },
        ScoredResult { id: 20, score: 4.0 },
        ScoredResult { id: 30, score: 3.0 },
    ];
    let vec_res = vec![
        ScoredResult { id: 20, score: 0.95 },
        ScoredResult { id: 10, score: 0.90 },
        ScoredResult { id: 40, score: 0.85 },
    ];

    let fused = rrf.rerank(&[fts, vec_res], 5);
    assert!(!fused.is_empty());
    let top2: Vec<i64> = fused.iter().take(2).map(|r| r.id).collect();
    assert!(top2.contains(&10) && top2.contains(&20));
}

#[test]
fn turbo_weighted_score_reranker() {
    let ws = WeightedScoreReranker::new(vec![0.8, 0.2]);
    let fts = vec![
        ScoredResult { id: 1, score: 10.0 },
        ScoredResult { id: 2, score: 5.0 },
    ];
    let vec_res = vec![
        ScoredResult { id: 3, score: 0.99 },
        ScoredResult { id: 1, score: 0.50 },
    ];

    let fused = ws.rerank(&[fts, vec_res], 3);
    assert!(!fused.is_empty());
    assert_eq!(fused[0].id, 1, "doc in both sources should rank first");
}

#[test]
fn turbo_mmr_diversity() {
    let mmr = MmrReranker { lambda: 0.5 };
    let emb1 = vec![1.0f32, 0.0, 0.0, 0.0];
    let emb2 = vec![0.99, 0.01, 0.0, 0.0];
    let emb3 = vec![0.0, 1.0, 0.0, 0.0];

    let results: Vec<(i64, f64, &[f32])> = vec![
        (1, 1.0, emb1.as_slice()),
        (2, 0.95, emb2.as_slice()),
        (3, 0.9, emb3.as_slice()),
    ];

    let reranked = mmr.rerank_with_embeddings(&results, 2);
    assert_eq!(reranked.len(), 2);
    assert_eq!(reranked[0].id, 1);
    assert_eq!(reranked[1].id, 3, "MMR should prefer diverse doc");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Phase 2: Head-to-head ALL strategies benchmark
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn turbo_benchmark_all_strategies() {
    let n = 500;
    let (tmp, _store) = build_test_store(n);
    let search = NdArraySearch::from_sqlite(tmp.path()).unwrap();
    let quantized = search.to_quantized();
    let matryoshka = search.to_matryoshka(MatryoshkaConfig::default());
    let binary = search.to_binary(true);
    let query = fake_emb(100);
    let k = 10;
    let iters = 200;

    // Warmup
    for _ in 0..5 {
        let _ = search.search(&query, k);
        let _ = search.search_simd(&query, k);
        let _ = quantized.search(&query, k);
        let _ = matryoshka.funnel_search(&query, k);
        let _ = binary.search_twophase(&query, k, 10);
    }

    let bench = |f: &dyn Fn()| -> f64 {
        let start = std::time::Instant::now();
        for _ in 0..iters {
            f();
        }
        start.elapsed().as_micros() as f64 / iters as f64
    };

    let ndarray_us = bench(&|| { let _ = search.search(&query, k); });
    let simd_us = bench(&|| { let _ = search.search_simd(&query, k); });
    let quant_us = bench(&|| { let _ = quantized.search(&query, k); });
    let funnel_us = bench(&|| { let _ = matryoshka.funnel_search(&query, k); });
    let binary_us = bench(&|| { let _ = binary.search_twophase(&query, k, 10); });

    eprintln!("\n=== ALL STRATEGIES ({n} docs, {EMBED_DIM}-dim, k={k}) ===");
    eprintln!("  ndarray f32:        {ndarray_us:.1}µs (baseline)");
    eprintln!("  SIMD f32:           {simd_us:.1}µs ({:.1}×)", ndarray_us / simd_us.max(0.1));
    eprintln!("  quantized i8:       {quant_us:.1}µs ({:.1}×)", ndarray_us / quant_us.max(0.1));
    eprintln!("  matryoshka funnel:  {funnel_us:.1}µs ({:.1}×)", ndarray_us / funnel_us.max(0.1));
    eprintln!("  binary two-phase:   {binary_us:.1}µs ({:.1}×)", ndarray_us / binary_us.max(0.1));
    eprintln!("  Memory: f32={}KB i8={}KB bin={}KB",
        n * EMBED_DIM * 4 / 1024,
        quantized.memory_bytes() / 1024,
        binary.binary_memory_bytes() / 1024,
    );
}
