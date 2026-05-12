# Wire `synapse-rerank` into LongMemEval — Plan

## Current state (verified 2026-05-10)

- `crates/synapse-rerank/src/`: `cascade.rs`, `colbert.rs`, `factory.rs`, `lib.rs`, `lightgbm.rs`, `clicklog.rs` — code present
- Cargo features: `onnx` (fastembed cross-encoder), `lightgbm` (LambdaMART) — **default OFF**
- `bench/longmemeval/longmemeval_adapter.rs` — JSONL parser + `answer_in_hits` metric only — **no recall pipeline**
- `bench/longmemeval/judge_server.py` — exists, presumably LLM-judge sidecar
- LongMemEval-S data: `bench/longmemeval/data/` (state unknown — check size)

## Gap

Adapter loads JSONL but never calls `synapse-core::recall()` → never invokes `synapse-rerank::cascade::cascade_rerank()`. So R@5=0.30 measurement uses unranked top-K.

## 3-step wire (≤1 day)

### Step 1 — runner binary `bench/longmemeval/src/main.rs`
```rust
use synapse_core::Brain;
use synapse_rerank::cascade::CascadeReranker;
use synapse_rerank::factory::Reranker;

fn main() -> anyhow::Result<()> {
    let examples = longmemeval_adapter::load_jsonl("data/longmemeval_s.jsonl")?;
    let brain = Brain::open(".synapse/longmem.db")?;
    let reranker = CascadeReranker::new()
        .with_lightgbm("models/lambdamart.txt")?      // top-500 → top-50
        .with_onnx("jinaai/jina-reranker-v2-base-multilingual")?; // top-50 → top-10

    // ingest haystacks once
    for ex in &examples {
        for s in &ex.haystack_sessions {
            for m in &s.messages {
                brain.put(&m.content, /*meta*/ &serde_json::json!({"sid": s.session_id}))?;
            }
        }
    }

    // recall + rerank
    let mut hit_at_5 = 0;
    for ex in &examples {
        let candidates = brain.hybrid(&ex.question, 500)?;
        let reranked = reranker.rerank(&ex.question, &candidates)?;
        let top5: Vec<&str> = reranked.iter().take(5).map(|h| h.text.as_str()).collect();
        if longmemeval_adapter::answer_in_hits(&ex.answer, &top5) { hit_at_5 += 1; }
    }
    println!("R@5 = {:.3}", hit_at_5 as f64 / examples.len() as f64);
    Ok(())
}
```

### Step 2 — Cargo.toml feature flag
```toml
[features]
default = []
rerank-full = ["synapse-rerank/onnx", "synapse-rerank/lightgbm"]
```

Build: `cargo run -p longmemeval-bench --features rerank-full --release`

### Step 3 — Bench gate

Run twice (variance), report median. Acceptance: **R@5 ≥ 0.65** (close half the gap to 0.85 with default models). With LambdaMART trained on accept-log → push to 0.80+.

## Cost

- Jina-reranker-v2-base ONNX: 278MB download, 40ms / 32-batch on M4 Max → 200ms per query for top-100 rerank → still <1s per example
- LightGBM model: ~100KB, 0.2ms inference

## Risk

- Model download on first run (no offline-bench possible without prefetched models)
- ONNX-runtime ARM64 wheel resolution (fastembed ≥4 covers this)
- LongMemEval data presence (`bench/longmemeval/data/`) — need to download from upstream first

## Next actions (sequenced)

1. `du -sh bench/longmemeval/data/` — verify dataset present, else `wget` upstream
2. Add `bench/longmemeval/Cargo.toml` runner crate (currently scaffold-only)
3. Implement `main.rs` per Step 1
4. `cargo run -p longmemeval-bench --features rerank-full --release 2>&1 | tee R5_2026-05-10.log`
5. If R@5 < 0.65 → enable LambdaMART training on synapse-learn click-log → re-bench
