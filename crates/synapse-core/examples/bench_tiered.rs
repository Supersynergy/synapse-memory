//! Quick bench: 200k docs — all-mem MultiIndex vs TieredIndex (threshold=100k).
//! Run: cargo run --example bench_tiered --features "spann-tier simsimd" --release

use std::time::Instant;
use synapse_core::turbo::multi_index::{MultiIndex, SearchHints};
use synapse_core::turbo::tiered::TieredIndex;

const N: usize = 200_000;
const DIM: usize = 384;
const K: usize = 10;

fn rand_vec(seed: u64, dim: usize) -> Vec<f32> {
    // xorshift for speed
    let mut s = seed.wrapping_add(1);
    let mut v: Vec<f32> = (0..dim)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s as f32) / (u64::MAX as f32) * 2.0 - 1.0
        })
        .collect();
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

fn main() {
    eprintln!("Generating {N} × {DIM}-dim docs …");
    let rows: Vec<(i64, Vec<f32>)> = (0..N as i64)
        .map(|i| (i, rand_vec(i as u64, DIM)))
        .collect();
    let query = rand_vec(999_999, DIM);

    // --- all-mem MultiIndex ---
    let t0 = Instant::now();
    let mem_idx = MultiIndex::build(rows.clone());
    let build_mem = t0.elapsed();
    let t1 = Instant::now();
    for _ in 0..100 {
        let _ = mem_idx.search(
            &query,
            SearchHints {
                k: K,
                ..Default::default()
            },
        );
    }
    let search_mem = t1.elapsed() / 100;
    eprintln!("all-mem  build={build_mem:.2?}  search(k={K})={search_mem:.2?}");

    // --- TieredIndex (threshold=100k, spills 100k to SPANN) ---
    let dir = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("SYNAPSE_RAM_THRESHOLD_DOCS", "100000") };
    let t2 = Instant::now();
    let mut tiered = TieredIndex::new(DIM, dir.path()).unwrap();
    for (id, v) in &rows {
        tiered.add(*id, v.clone()).unwrap();
    }
    // force flush
    tiered.flush_disk().unwrap();
    let build_tiered = t2.elapsed();
    let t3 = Instant::now();
    for _ in 0..100 {
        let _ = tiered.search(&query, K);
    }
    let search_tiered = t3.elapsed() / 100;
    eprintln!(
        "tiered   build={build_tiered:.2?}  search(k={K})={search_tiered:.2?}  (100k mem + 100k SPANN)"
    );
    eprintln!(
        "search latency Δ: tiered is {:.1}× vs all-mem",
        search_tiered.as_secs_f64() / search_mem.as_secs_f64()
    );
}
