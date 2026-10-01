//! Pure-Rust ndarray_search benchmark with real corpus
//!
//! Compares:
//!   - ndarray (BLAS/Accelerate) — what the FIXED normalize_rows enables
//!   - simsimd (NEON) — when simsimd feature is enabled
use std::time::Instant;

fn main() {
    let brain_db = std::path::Path::new("/Users/master/.synapse/brain.db");

    println!("Loading corpus from {}...", brain_db.display());

    // Register sqlite-vec extension BEFORE opening the connection
    // (auto_extension registers it globally in the SQLite library)
    unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    }

    // Test 1: Direct rusqlite connection (works)
    let conn = rusqlite::Connection::open(&brain_db).expect("open db");
    let t0 = Instant::now();
    println!("Test 1: Direct rusqlite Connection...");
    let search = synapse_core::turbo::ndarray_search::NdArraySearch::from_connection(&conn)
        .expect("failed to load");
    let load_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let n = search.len();
    println!(
        "  Direct connection: {} vectors loaded in {:.0}ms",
        n, load_ms
    );

    // Use a deterministic query vector
    let query: Vec<f32> = (0..384).map(|i| ((i as f32 * 0.01).sin())).collect();

    // ── ndarray search ──
    println!("\n--- ndarray (BLAS/Accelerate) ---");
    let mut t_ndarray = Vec::with_capacity(20);
    for i in 0..25 {
        let t = Instant::now();
        let r = search.search(&query, 10);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        if i >= 5 {
            t_ndarray.push(ms);
        }
        if i == 5 {
            println!("  results: {:?}", r.iter().take(3).collect::<Vec<_>>());
        }
    }
    t_ndarray.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = t_ndarray[t_ndarray.len() / 2];
    let p95 = t_ndarray[(t_ndarray.len() as f64 * 0.95) as usize];
    println!(
        "  p50={}ms  p95={}ms",
        format!("{:.2}", p50),
        format!("{:.2}", p95)
    );

    // ── simsimd search ──
    #[cfg(feature = "simsimd")]
    {
        println!("\n--- simsimd (NEON) ---");
        let mut t_simd = Vec::with_capacity(20);
        for i in 0..25 {
            let t = Instant::now();
            let _r = search.search_simsimd(&query, 10);
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            if i >= 5 {
                t_simd.push(ms);
            }
        }
        t_simd.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p50s = t_simd[t_simd.len() / 2];
        let p95s = t_simd[(t_simd.len() as f64 * 0.95) as usize];
        println!(
            "  p50={}ms  p95={}ms",
            format!("{:.2}", p50s),
            format!("{:.2}", p95s)
        );
        println!("  ndarray vs simsimd: {}x", format!("{:.1}", p50 / p50s));
    }

    // Test 2: Via Store::open (was hanging)
    drop(search);
    drop(conn);

    println!("\n=== Test 2: Via Store::open ===");
    let t1 = Instant::now();
    let store = synapse_core::Store::open(&brain_db).expect("open store");
    println!("  Store opened in {:.1}s", t1.elapsed().as_secs_f64());

    let t2 = Instant::now();
    println!("  Calling warm_turbo...");
    store.warm_turbo();
    println!("  warm_turbo done in {:.1}s", t2.elapsed().as_secs_f64());

    println!("\nAll tests passed!");
}
