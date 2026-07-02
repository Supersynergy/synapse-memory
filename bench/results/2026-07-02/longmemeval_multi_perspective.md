# LongMemEval multi-perspective benchmark — 2026-07-02

Change under test: official LongMemEval JSON support, Evidence-R@5/R@10 reporting, and rerank candidate-pool expansion before final top-10 truncation.

## Environment

- Repo: `/Users/master/BASE/projects/synapse`
- Dataset used: `bench/longmemeval/data/lme_s_50.json` legacy 50-question subset
- Official Full-500 dataset: not present locally, so Evidence-R metrics were code-verified but not score-reported in these runs.
- Build gate: `cargo check -p longmemeval --no-default-features` passed before and after the `--raw-query` fix.

## Results

| Perspective | Command | N | Errors | Recall@5 | Recall@10 | Fuzzy-R@5 | Fuzzy-R@10 | Latency avg |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| Legacy lexical baseline | `cargo run -p longmemeval --no-default-features -- --rerank-top 0` | 50 | 0 | 0.640 (32/50) | 0.640 (32/50) | 0.600 (30/50) | 0.620 (31/50) | 3.10 ms |
| Raw-query ablation (post-fix) | `cargo run -p longmemeval --no-default-features -- --rerank-top 0 --raw-query` | 50 | 0 | 0.640 (32/50) | 0.640 (32/50) | 0.600 (30/50) | 0.620 (31/50) | 1.00 ms |
| Rule pre-extract ablation | `cargo run -p longmemeval --no-default-features -- --rerank-top 0 --pre-extract` | 50 | 0 | 0.560 (28/50) | 0.640 (32/50) | 0.540 (27/50) | 0.620 (31/50) | 1.23 ms |
| BGE rerank smoke | `cargo run -p longmemeval -- --rerank-top 20 --limit 5` | 5 | 0 | 0.400 (2/5) | 0.400 (2/5) | 0.600 (3/5) | 0.600 (3/5) | 4222.98 ms |

## Notes

- The full BGE rerank run (`cargo run -p longmemeval -- --rerank-top 20`) compiled and initialized `BGE-reranker-v2-m3`, then was killed before completion on the local machine.
- The bounded rerank smoke verifies the changed rerank-pool code path without turning the benchmark into an OOM/runtime risk.
- The raw-query run exposed a review issue: `--raw-query` was parsed but not passed into `run_question`; after the fix, the focused post-fix run produced the same recall scores with 1.00 ms average recall latency.
- The legacy subset has no `answer_session_ids`, so Evidence-R output is intentionally absent for those runs.

## Raw log

Raw terminal output was captured at `bench/results/2026-07-02/longmemeval_multi_perspective.log` during the run.
