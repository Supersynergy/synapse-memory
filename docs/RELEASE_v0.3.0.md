# Synapse v0.3.0 — Self-Learning

## New Features

### `crates/synapse-learn` (new crate)
- **`bandit.rs`** — Thompson-sampling shard router. `ShardBandit` maintains per-shard Beta(wins, losses) priors; `pick_shard` samples argmax, `reward` updates priors. Persisted in `learn_bandit` table.
- **`rrf_tune.rs`** — Adaptive hybrid-RRF-alpha. Per query-shape-hash (first-token-len + has-digit + has-quote), bandit over 5 alpha-buckets [0.0, 0.25, 0.5, 0.75, 1.0]. Table: `learn_rrf_alpha`.
- **`feedback.rs`** — `synapse feedback <query_id> <doc_id> [--shard-id]` logs to `feedback` table and calls `bandit::reward(shard, true)`.
- **`heat.rs`** — On search hits, increments `docs.access_count` + `last_accessed_ts`. Rerank: `score *= (1 + ln(1+access)) * exp(-0.05 * age_days)`.
- **`drift.rs`** — Samples up to 100 docs, re-embeds, computes mean cosine sim. WARN <0.98, ERROR <0.95.
- **`consolidate.rs`** — LSH (8 random-projection planes) over 384-dim embeddings, candidate pairs in same bucket with cosine>0.95 are merged (duplicate marked in meta JSON).
- **`calibrate.rs`** — Platt-scaling bucket table `learn_calibration`; `calibrate(raw_score)` applies per-bucket correction.

### CLI additions
- `synapse learn status` — shows bandit shard count + feedback entry count
- `synapse learn consolidate` — runs near-dup merge
- `synapse learn drift-check` — checks embedding drift (requires embedder)
- `synapse learn calibrate` — updates calibration from feedback log
- `synapse feedback <query_id> <doc_id>` — record positive feedback

## Migration Notes
- No schema changes to existing `brain.db` files (heat columns added lazily via `heat::migrate_heat`).
- Learning state stored in `<brain>.learn.db` alongside the main DB.
- `--no-bandit` flag for shard-router opt-out planned for v0.3.1.

## Test Results
- `cargo test -p synapse-learn`: 8/8 pass
- `bench/e2e_smoke.sh`: 23/23 pass (3 new tests: 11a-c)
