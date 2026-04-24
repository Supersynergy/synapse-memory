use std::sync::{Arc, Mutex};
use std::thread;
use synapse_core::{Store, types::{PutRequest, SearchMode}};

fn bench_scale(path: &str, n: usize) -> (f64, f64, f64) {
    let _ = std::fs::remove_file(path);
    let mut store = Store::open(path).unwrap();

    // put benchmark
    let t0 = std::time::Instant::now();
    for i in 0..n {
        store.put(&PutRequest {
            text: format!("document {i} contains searchable content about topic {}", i % 100),
            uri: Some(format!("doc://{i}")),
            title: Some(format!("Doc {i}")),
            embedding: None,
            meta: None,
        }).unwrap();
    }
    let put_us = t0.elapsed().as_micros() as f64 / n as f64;

    // lex search benchmark (100 iters, capped for large corpora)
    let iters = 100usize;
    let t0 = std::time::Instant::now();
    for i in 0..iters {
        let q = format!("topic {}", i % 100);
        let _ = store.search(&q, SearchMode::Lex, None, 10).unwrap();
    }
    let lex_us = t0.elapsed().as_micros() as f64 / iters as f64;

    // vec search benchmark
    let q_vec: Vec<f32> = (0..384).map(|i| (i as f32 * 0.001).sin()).collect();
    let t0 = std::time::Instant::now();
    for _ in 0..iters {
        let _ = store.search("", SearchMode::Vec, Some(&q_vec), 10).unwrap();
    }
    let vec_us = t0.elapsed().as_micros() as f64 / iters as f64;

    (put_us, lex_us, vec_us)
}

fn bench_concurrent_readers(path: &str, threads: usize) -> f64 {
    // path already populated with 100k docs from scale ladder
    let store = Arc::new(Mutex::new(Store::open(path).unwrap()));
    let q_vec: Vec<f32> = (0..384).map(|i| (i as f32 * 0.001).sin()).collect();
    let iters_per_thread = 50usize;

    let t0 = std::time::Instant::now();
    let handles: Vec<_> = (0..threads).map(|_| {
        let store = Arc::clone(&store);
        let q_vec = q_vec.clone();
        thread::spawn(move || {
            for _ in 0..iters_per_thread {
                let s = store.lock().unwrap();
                let _ = s.search("", SearchMode::Vec, Some(&q_vec), 10).unwrap();
            }
        })
    }).collect();
    for h in handles { h.join().unwrap(); }

    let total_ops = (threads * iters_per_thread) as f64;
    t0.elapsed().as_micros() as f64 / total_ops
}

fn main() {
    println!("=== synapse library-mode scale ladder ===\n");

    // Scale ladder
    let scales: &[(usize, &str)] = &[
        (1_000,     "/tmp/lib-demo-1k.db"),
        (10_000,    "/tmp/lib-demo-10k.db"),
        (100_000,   "/tmp/lib-demo-100k.db"),
        (1_000_000, "/tmp/lib-demo-1m.db"),
    ];

    println!("{:<10} {:>10} {:>10} {:>10}", "docs", "put_µs", "lex_µs", "vec_µs");
    println!("{}", "-".repeat(44));

    let mut path_100k = "";
    for (n, path) in scales {
        let (put_us, lex_us, vec_us) = bench_scale(path, *n);
        println!("{:<10} {:>10.1} {:>10.1} {:>10.1}", n, put_us, lex_us, vec_us);
        if *n == 100_000 { path_100k = path; }
    }

    // Concurrent reader test at 100k
    println!("\n=== concurrent reader test @ 100k docs ===\n");
    println!("{:<10} {:>14}", "threads", "vec_µs/op (mutex)");
    println!("{}", "-".repeat(28));
    for t in [4, 8, 16] {
        // Reopen for reader test (store already populated from scale ladder)
        let avg = bench_concurrent_readers(path_100k, t);
        println!("{:<10} {:>14.1}", t, avg);
    }

    // MCP baseline for reference
    let mcp_us = 3_450.0f64;
    println!("\nMCP baseline: {mcp_us:.0}µs (fixed IPC overhead, thread-independent)");
}
