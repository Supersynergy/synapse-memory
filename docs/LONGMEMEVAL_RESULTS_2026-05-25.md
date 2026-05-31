# LongMemEval-S Baseline - 2026-05-25

## Scope

This is a no-download release baseline for the Synapse Context OS memory
pipeline. It uses the checked-in LongMemEval-S-50 subset and disables model
reranking so the result is reproducible without Hugging Face model downloads.

Dataset:

- `bench/longmemeval/data/lme_s_50.json`
- 50 questions
- average 47.3 session documents per question

Primary metric:

- substring `Recall@5`
- substring `Recall@10`

Diagnostic metric:

- token-set `Fuzzy-R@5`
- token-set `Fuzzy-R@10`

## Command

```bash
cargo run -p longmemeval --no-default-features -- --rerank-top 0
```

## Result

```text
LongMemEval-S bench: 50 questions | hooks: Rule | floor=0 hyde_th=3
--- Results ---
N            : 50
Errors       : 0
Recall@5     : 0.640  (32/50)
Recall@10    : 0.640  (32/50)
Fuzzy-R@5    : 0.600  (30/50)  [token-set 0.6]
Fuzzy-R@10   : 0.620  (31/50)  [token-set 0.6]
Latency avg  : 0.97 ms (recall only, ingest excluded)
Avg docs/Q   : 47.3
```

Smoke run before the full baseline:

```bash
cargo run -p longmemeval --no-default-features -- --limit 5 --rerank-top 0
```

```text
N            : 5
Errors       : 0
Recall@5     : 0.400  (2/5)
Recall@10    : 0.400  (2/5)
Fuzzy-R@5    : 0.600  (3/5)
Fuzzy-R@10   : 0.600  (3/5)
Latency avg  : 1.06 ms (recall only, ingest excluded)
Avg docs/Q   : 47.2
```

## Interpretation

- The release has a reproducible LongMemEval-S quality floor: `Recall@5=0.640`
  on the 50-question subset with no embedding model and no reranker download.
- This validates the current deterministic recall path, not the optional
  embedding/reranker path.
- The latency number is recall-only. It excludes per-question ingest and
  migration work performed by the benchmark harness.
- Fuzzy recall is included for diagnosis only. Release claims should use
  substring recall unless a judge-based protocol is explicitly run.

## Non-Claims

- LoCoMo was not run.
- LLM-judge mode was not run.
- ONNX cross-encoder reranking was not run in this verification pass.
- MLX/MiniMax extraction was not run in this verification pass.
- Do not claim broad memory SOTA from this single subset result.

## Next Gates

1. Run the optional reranker baseline with cached or explicitly downloaded
   models.
2. Run judge mode for protocol parity with LongMemEval paper scoring.
3. Add LoCoMo or another multi-session memory benchmark before claiming
   general memory quality leadership.
