use synapse_core::{Store, types::{PutRequest, SearchMode}};

fn main() {
    let path = "/tmp/lib-demo.db";
    let _ = std::fs::remove_file(path);
    let mut store = Store::open(path).unwrap();

    // ── put benchmark ─────────────────────────────────────────────────────
    let n = 10_000usize;
    let t0 = std::time::Instant::now();
    for i in 0..n {
        store.put(&PutRequest {
            text: format!("document {i} contains some searchable content about topic {}", i % 100),
            uri: Some(format!("doc://{i}")),
            title: Some(format!("Doc {i}")),
            embedding: None,
            meta: None,
        }).unwrap();
    }
    let put_us = t0.elapsed().as_micros() as f64 / n as f64;

    // ── lex search benchmark ──────────────────────────────────────────────
    let iters = 1_000usize;
    let t0 = std::time::Instant::now();
    for i in 0..iters {
        let q = format!("topic {}", i % 100);
        let _ = store.search(&q, SearchMode::Lex, None, 10).unwrap();
    }
    let search_us = t0.elapsed().as_micros() as f64 / iters as f64;

    // ── vec search benchmark (random query vec, 384-dim) ──────────────────
    let q_vec: Vec<f32> = (0..384).map(|i| (i as f32 * 0.001).sin()).collect();
    let t0 = std::time::Instant::now();
    for _ in 0..iters {
        let _ = store.search("", SearchMode::Vec, Some(&q_vec), 10).unwrap();
    }
    let vec_search_us = t0.elapsed().as_micros() as f64 / iters as f64;

    // MCP-mode baseline (from existing bench docs): 3450 µs round-trip
    let mcp_us = 3_450.0f64;
    let put_speedup = mcp_us / put_us;
    let lex_speedup = mcp_us / search_us;
    let vec_speedup = mcp_us / vec_search_us;

    println!("=== synapse library-mode (PIONEER P1) ===");
    println!("put_us       = {put_us:.2}  ({put_speedup:.0}× faster than MCP {mcp_us:.0}µs)");
    println!("lex_search_us = {search_us:.2}  ({lex_speedup:.0}× faster than MCP)");
    println!("vec_search_us = {vec_search_us:.2}  ({vec_speedup:.0}× faster than MCP)");
    println!("docs inserted = {n}");
}
