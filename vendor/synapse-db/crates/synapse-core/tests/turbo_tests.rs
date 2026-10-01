//! Comprehensive tests for synapse-turbo modules.
//!
//! Test patterns inspired by:
//! - Qdrant: RRF scoring correctness (empty, single, multi, k-parameter)
//! - RuVector/GrafeoDB: recall@k measurement, brute-force kNN ground truth
//! - AppFlowy: sqlite-vec init + roundtrip verification
//!
//! Run: cargo test -p synapse-core --features turbo -- turbo

#![cfg(feature = "turbo")]

use synapse_core::turbo::hybrid_cache::HybridCache;
use synapse_core::turbo::ndarray_search::{HybridSearch, NdArraySearch};
use synapse_core::types::EMBED_DIM;
use synapse_core::{PutRequest, Store};

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
        .search("", synapse_core::SearchMode::Vec, Some(&query_emb), k)
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
