//! mmap SIFT-corpus bench: compare cold-load (Vec) vs mmap loading time.
//!
//! Usage (no actual SIFT file needed — generates synthetic corpus):
//!   cargo run -p synapse-ann --release --features ann-usearch --example mmap_sift_bench
//!
//! With real SIFT-1M base vectors (bvecs format):
//!   SIFT_PATH=/data/sift/sift_base.fvecs cargo run ...
//!
//! Measures:
//!   - Heap-alloc load time (Vec<Vec<f32>>) vs mmap + MADV_WILLNEED prefetch
//!   - HNSW build time on 100k synthetic f32 vectors
//!   - Batch-search QPS single vs parallel (if ann-batch feature present)

#[cfg(not(feature = "ann-usearch"))]
fn main() {
    eprintln!("Requires --features ann-usearch");
    std::process::exit(1);
}

#[cfg(feature = "ann-usearch")]
fn main() {
    use std::io::Write as _;
    use synapse_ann::AnnIndex as _;
    use synapse_ann::usearch_backend::UsearchIndex;

    let n: usize = std::env::var("N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100_000);
    let dim: usize = std::env::var("DIM")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(128);
    let q_count: usize = 64;
    let k = 10;

    println!("=== mmap_sift_bench n={n} dim={dim} ===");

    // ── 1. Write synthetic fvecs to a temp file ──────────────────────────────
    let tmp = std::env::temp_dir().join("synapse_sift_bench.fvecs");
    {
        let t = std::time::Instant::now();
        let mut f = std::fs::File::create(&tmp).unwrap();
        for i in 0..n as u64 {
            let v = synthetic_vec(i, dim);
            let dim_u32 = dim as u32;
            f.write_all(&dim_u32.to_le_bytes()).unwrap();
            for x in &v {
                f.write_all(&x.to_le_bytes()).unwrap();
            }
        }
        println!(
            "fvecs write {n} vecs: {:.1}ms",
            t.elapsed().as_secs_f64() * 1e3
        );
    }

    let bytes_per_vec = 4 + dim * 4; // dim_u32 header + floats
    let expected_bytes = n * bytes_per_vec;

    // ── 2. Heap-alloc load (baseline) ────────────────────────────────────────
    let heap_vecs: Vec<Vec<f32>> = {
        let t = std::time::Instant::now();
        let raw = std::fs::read(&tmp).unwrap();
        let mut out = Vec::with_capacity(n);
        let mut pos = 0;
        while pos + bytes_per_vec <= raw.len() {
            let d = u32::from_le_bytes(raw[pos..pos + 4].try_into().unwrap()) as usize;
            pos += 4;
            let floats: Vec<f32> = (0..d)
                .map(|j| f32::from_le_bytes(raw[pos + j * 4..pos + j * 4 + 4].try_into().unwrap()))
                .collect();
            pos += d * 4;
            out.push(floats);
        }
        println!(
            "heap load {}: {:.1}ms",
            out.len(),
            t.elapsed().as_secs_f64() * 1e3
        );
        out
    };

    // ── 3. mmap load + MADV_WILLNEED prefetch ────────────────────────────────
    #[cfg(target_family = "unix")]
    let _mmap_load_ms = {
        use std::os::unix::fs::OpenOptionsExt as _;
        let t = std::time::Instant::now();
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_RDONLY)
            .open(&tmp)
            .unwrap();
        let mmap = unsafe { memmap2::MmapOptions::new().map(&file).unwrap() };
        // MADV_WILLNEED: hint kernel to prefetch pages.
        #[cfg(target_os = "macos")]
        unsafe {
            libc::madvise(mmap.as_ptr() as *mut _, mmap.len(), libc::MADV_WILLNEED);
        }
        #[cfg(target_os = "linux")]
        unsafe {
            libc::madvise(mmap.as_ptr() as *mut _, mmap.len(), libc::MADV_WILLNEED);
        }
        // Parse directly from mmap'd memory.
        let mut count = 0usize;
        let mut pos = 0usize;
        let raw: &[u8] = &mmap;
        while pos + bytes_per_vec <= raw.len() {
            let d = u32::from_le_bytes(raw[pos..pos + 4].try_into().unwrap()) as usize;
            pos += 4 + d * 4;
            count += 1;
        }
        let ms = t.elapsed().as_secs_f64() * 1e3;
        println!("mmap load {count}: {ms:.1}ms");
        drop(mmap);
        ms
    };

    #[cfg(not(target_family = "unix"))]
    let _mmap_load_ms = {
        println!("mmap: not unix, skipping");
        0.0f64
    };

    // ── 4. HNSW build ────────────────────────────────────────────────────────
    let mut idx = UsearchIndex::new(dim, n).unwrap();
    let t = std::time::Instant::now();
    for (i, v) in heap_vecs.iter().enumerate() {
        idx.insert(i as u64, v).unwrap();
    }
    let build_ms = t.elapsed().as_secs_f64() * 1e3;
    println!(
        "HNSW build {n}: {build_ms:.1}ms  ({:.0} vecs/ms)",
        n as f64 / build_ms
    );

    // ── 5. Single-query baseline ──────────────────────────────────────────────
    let queries: Vec<Vec<f32>> = (0..q_count as u64)
        .map(|i| synthetic_vec(i * 9973, dim))
        .collect();

    let t = std::time::Instant::now();
    for q in &queries {
        let _ = idx.search(q, k).unwrap();
    }
    let single_ms = t.elapsed().as_secs_f64() * 1e3;
    let single_qps = q_count as f64 / (single_ms / 1e3);
    println!("single-query {q_count}q: {single_ms:.2}ms  {single_qps:.0} QPS");

    // ── 6. Batch (rayon) ─────────────────────────────────────────────────────
    #[cfg(feature = "ann-batch")]
    {
        let t = std::time::Instant::now();
        let _ = idx.search_batch(&queries, k);
        let batch_ms = t.elapsed().as_secs_f64() * 1e3;
        let batch_qps = q_count as f64 / (batch_ms / 1e3);
        println!(
            "batch-search {q_count}q: {batch_ms:.2}ms  {batch_qps:.0} QPS  speedup={:.2}×",
            batch_qps / single_qps
        );
        // Run 2 for variance.
        let t = std::time::Instant::now();
        let _ = idx.search_batch(&queries, k);
        let batch_ms2 = t.elapsed().as_secs_f64() * 1e3;
        println!(
            "batch-search run-2: {batch_ms2:.2}ms  speedup={:.2}×",
            single_ms / batch_ms2
        );
    }
    #[cfg(not(feature = "ann-batch"))]
    println!("batch-search: enable --features ann-batch");

    // ── 7. Summary ────────────────────────────────────────────────────────────
    println!("\n=== Summary ===");
    println!("corpus size:   {n} × {dim}f32");
    println!("HNSW build:    {build_ms:.1}ms");
    println!("single QPS:    {single_qps:.0}");

    let _ = std::fs::remove_file(&tmp);
    let _ = expected_bytes;
}

fn synthetic_vec(seed: u64, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|i| {
            let mix = seed
                .wrapping_mul(0x9E3779B97F4A7C15)
                .wrapping_add(i as u64)
                .wrapping_mul(0xBF58476D1CE4E5B9);
            (mix as i32 as f32) / (i32::MAX as f32)
        })
        .collect()
}
