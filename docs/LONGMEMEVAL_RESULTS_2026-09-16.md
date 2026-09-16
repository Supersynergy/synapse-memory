# LongMemEval-S Baseline — 2026-09-16 (rc.2 + post-release fixes)

Fresh reproduction of the no-download release baseline on current code
(`1.0.1-rc.2` + `2035a96b` WAL fix + `3ff7cf9d` bench artifact). Metrics are
identical to the 2026-05-25 run — the pipeline is deterministic.

Dataset:

- `bench/longmemeval/data/lme_s_50.json` — 50 questions, ~47.3 session docs/Q
- RuleHooks, no embedding model, no reranker, no LLM judge → zero downloads

## Command

```bash
cargo run -p longmemeval --no-default-features --release -- --rerank-top 0
```

## Result

```text
N            : 50
Errors       : 0
Recall@5     : 0.640  (32/50)
Recall@10    : 0.640  (32/50)
Fuzzy-R@5    : 0.600  (30/50)  [token-set 0.6]
Fuzzy-R@10   : 0.620  (31/50)  [token-set 0.6]
Latency avg  : 0.79 ms (recall only, ingest excluded)
Avg docs/Q   : 47.3
```

## Claim boundary

- Citable: substring/fuzzy R@k on the fixed 50-question subset, zero errors,
  sub-ms recall latency. Verified twice: 2026-05-25 and 2026-09-16.
- NOT established: judge-mode scores (`--judge` needs MLX models), full
  LongMemEval-S/M (500/500+ questions — needs the official dataset), and
  evidence-session R@k (the subset carries no `answer_session_ids` labels).
  Do not cite those until the corresponding run is recorded here.
