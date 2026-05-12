//! Bench: BMP vs naive SPLADE search
//! 10k docs, 100 queries, top-10.
//! Run: cargo bench -p synapse-splade

use std::hint::black_box;
use std::time::Instant;
use synapse_splade::{BlockMaxIndex, SpladeEncoder, SpladeIndex};

fn main() {
    let enc = SpladeEncoder::default();
    const N_DOCS: usize = 10_000;
    const N_QUERIES: usize = 100;
    const TOP_K: usize = 10;

    // Build indexes
    let mut naive = SpladeIndex::open(":memory:").unwrap();
    let mut bmp = BlockMaxIndex::default();

    let templates = [
        "splade neural sparse retrieval model transformer",
        "dense retrieval bi-encoder sentence embedding",
        "inverted index posting list BM25 ranking",
        "transformer masked language model BERT pretraining",
        "query expansion pseudo relevance feedback",
        "colbert late interaction multi-vector retrieval",
        "sparse representation regularisation vocabulary",
        "neural ranking passage reranking cross-encoder",
        "MTEB benchmark retrieval recall evaluation",
        "knowledge graph entity linking relation extraction",
    ];

    println!("Indexing {} docs...", N_DOCS);
    for i in 0..N_DOCS {
        let text = format!("{} doc_{}", templates[i % templates.len()], i);
        let sv = enc.encode(&text).unwrap();
        naive.add_doc(i as u64, &sv).unwrap();
        bmp.add_doc(i as u64, &sv);
    }
    bmp.flush();

    // Build queries
    let queries: Vec<_> = (0..N_QUERIES)
        .map(|i| enc.encode(&format!("{} query_{}", templates[i % templates.len()], i)).unwrap())
        .collect();

    // Warm up
    for q in &queries {
        let _ = black_box(naive.search(q, TOP_K).unwrap());
        let _ = black_box(bmp.search_topk(q, TOP_K));
    }

    // Bench naive
    let t0 = Instant::now();
    for _ in 0..5 {
        for q in &queries {
            black_box(naive.search(q, TOP_K).unwrap());
        }
    }
    let naive_ms = t0.elapsed().as_secs_f64() * 1000.0 / 5.0;

    // Bench BMP
    let t1 = Instant::now();
    for _ in 0..5 {
        for q in &queries {
            black_box(bmp.search_topk(q, TOP_K));
        }
    }
    let bmp_ms = t1.elapsed().as_secs_f64() * 1000.0 / 5.0;

    let speedup = naive_ms / bmp_ms;
    println!("Naive  : {:.2}ms / 100 queries", naive_ms);
    println!("BMP    : {:.2}ms / 100 queries", bmp_ms);
    println!("Speedup: {:.2}×", speedup);

    // Rank equivalence spot-check
    let mut rank_eq_ok = 0usize;
    for q in &queries {
        let naive_ids: Vec<u64> = naive.search(q, TOP_K).unwrap().into_iter().map(|(id,_)| id).collect();
        let bmp_ids: Vec<u64> = bmp.search_topk(q, TOP_K).into_iter().map(|(id,_)| id).collect();
        if naive_ids == bmp_ids { rank_eq_ok += 1; }
    }
    println!("Rank-eq: {}/{} queries match", rank_eq_ok, N_QUERIES);

    if speedup < 2.0 {
        eprintln!("WARNING: speedup {:.2}× < 2× target", speedup);
        std::process::exit(1);
    }
}
