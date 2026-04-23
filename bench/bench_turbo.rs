//! Benchmark for synapse-turbo optimizations
//!
//! Run: cargo bench --features turbo --bench bench_turbo

use synapse_core::turbo::{ndarray_search::NdArraySearch, hybrid_cache::HybridCache};
use std::time::Instant;

fn bench_ndarray_search(brain_path: &str, queries: &[&str]) {
    println!("\n=== NdArray Search Benchmark ===");
    
    let search = NdArraySearch::from_sqlite(brain_path).unwrap();
    println!("Loaded {} vectors", search.len());
    
    // Generate random query embeddings (for benchmarking)
    let dummy_query = vec![0.1f32; 384];
    
    // Warmup
    for _ in 0..10 {
        let _ = search.search(&dummy_query, 10);
    }
    
    // Benchmark
    let iterations = 1000;
    let t0 = Instant::now();
    for _ in 0..iterations {
        let _ = search.search(&dummy_query, 10);
    }
    let elapsed = t0.elapsed().as_secs_f64() * 1000.0;
    println!("{} searches: {:.3}ms ({:.3}ms per search)", 
             iterations, elapsed, elapsed / iterations as f64);
}

fn bench_hybrid_cache() {
    println!("\n=== Hybrid Cache Benchmark ===");
    
    let cache = HybridCache::new().unwrap();
    
    // Insert embeddings
    let emb = vec![0.1f32; 384];
    for i in 0..100 {
        cache.put_embedding(&format!("query_{}", i), &emb);
    }
    
    // Benchmark lookups
    let iterations = 10000;
    let t0 = Instant::now();
    for i in 0..iterations {
        let _ = cache.get_embedding(&format!("query_{}", i % 100));
    }
    let elapsed = t0.elapsed().as_secs_f64() * 1000.0;
    println!("{} cache lookups: {:.3}ms ({:.3}μs per lookup)",
             iterations, elapsed, elapsed / iterations as f64 * 1000.0);
}

fn main() {
    println!("synapse-turbo benchmark");
    
    // These would use real paths in actual benchmark
    // bench_ndarray_search("/path/to/brain.db", &["MiniMax", "agno"]);
    // bench_hybrid_cache();
    
    println!("\nTo run full benchmark:");
    println!("  cargo bench --features turbo,embed --bench bench_turbo");
}
