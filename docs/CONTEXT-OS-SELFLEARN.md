# Synapse Context-OS — Self-Learning, Token-Budget-Aware

> Status: design 2026-05-31. Grounded in what already exists in this repo.
> Reframe: stop answering "docs similar to query". Start answering
> "given THIS agent + THIS task + a TOKEN BUDGET of N, return the minimal
> STATE that maximizes task success — and learn from whether it helped."

## What already exists (do NOT rebuild)
- Retrieval: hybrid RRF 8ms, MRL/f16/Hamming, HNSW ANN, ColBERT + SPLADE late-interaction (`synapse-fusion`, `synapse-ann`, `synapse-colbert`, `synapse-splade`).
- Rank: `synapse-rerank` (ms-marco-MiniLM), `synapse-learn/rrf_tune` (learns RRF weights), source-trust-prior table (`tuning.md`: known-fact +0.022).
- Learn: `synapse-learn` = bandit (Beta), calibrate, consolidate, drift, feedback, heat, query_log (learn-to-rank), rrf_tune. `synapse-tune` = TabPFN/Heuristic tuner, TtlBandit, BotClassifier, DriftDetector.
- Loop: `feedback.rs::record_accept` -> reward_context -> update_bandit; `sweep_unaccepted` infers misses. `synx context` + `synx ground` (hybrid seeds -> PageRank -> traverse -> JSON bundle).
- Direction already set: `decision/synapse-goals-context-os-2026-05-25` = local-first Context OS for agents, not DB-feature sprawl.

Conclusion: Synapse is ~70% a self-learning context engine. The gaps are (a) a token-budget-aware PACK layer, (b) a real reward signal, (c) a current-state ledger.

## The 5 layers
- L0 Ingest/Normalize (HAVE): put/import, blake3 dedup, FTS5+vec0. Add: every doc tagged `kind` (known-fact/decision/file/chat/corpus) -> drives trust-prior.
- L1 Retrieve (HAVE): hybrid RRF. No major gap.
- L2 Rank (PARTIAL): rrf_tune + rerank + trust-prior. Gap: warm-load rerank model (tuning.md #5), per-query-type learned weights (query_log).
- L3 PACK (BIG GAP — this IS the Superverschnellerung): given budget B tokens:
  1. near-dup collapse (MinHash/LSH, invariant #17) — never pay twice for one fact.
  2. tiered compress per candidate: full -> signatures -> fact-delta -> 1-line. Knapsack-select tier per remaining budget (invariant: move work to cheapest tier).
  3. order by serial-position: best first + last, weak in middle.
  4. emit STATE card: what is known / what changed since last turn / what is still open.
- L4 Feedback/Learn (PARTIAL -> close loop): today only explicit accept. Evolve to implicit signals (cited-doc-ids + turn verify-gate pass/fail), per decision/feedback-rerank. Reward trains: RRF weights, trust-priors, compression-tier policy, budget split.

## Token-savers from RTK / context-mode / caveman — evolved, not copied
- RTK (static 60-90% output filter) -> LEARNED compressor that knows which tokens carry signal for THIS query-kind (entropy bound, invariant #11).
- context-mode (FTS5 keyword pre-filter) -> already have hybrid; add the budget-knapsack packer on top.
- caveman (drop articles/filler) -> the L3 fact-delta tier IS caveman-for-retrieval: STATE not narrative, stored and returned.

## "Always knows state"
- `consolidate.rs` + supersession links on every writeback. New `synx state <topic>` returns CURRENT truth + what superseded what + open questions (append-only ledger + supersession, invariants #56/#66).
- Pattern already emerging manually (e.g. superweb-botwall trail 311297->311312->311340->full-picture). Automate it.

## Self-learning glue
- Thompson bandit (already in synapse-learn/tune) picks per query: retriever mix, rerank weight-set, compression tier, budget split.
- Reward = task-gate-pass AND tokens-saved (joint objective — quality must gate cost).
- Active-learning: only re-tune on informative queries (invariant #81). ASHA halving for sample-efficient sweeps (autolearn pattern).

## Phased plan
- P0 (done 2026-05-31): ingest ~/SpeedTuning + ~/machinelearning corpora (50 docs), write this SPEC + known-fact.
- P1 REWARD SIGNAL (highest leverage, smallest code): `synx feedback --used <doc_ids> --gate pass|fail`; CC turn-end hook calls it. Trains existing bandit/rrf_tune from a REAL signal. ~1-2 files in synapse-learn + 1 hook. <- build first.
- P2 PACKER crate `synapse-pack`: budget-knapsack + tiered compressor + MinHash dedup + serial-position order. Wraps `synx ground`. ~4-5 files.
- P3 compression-tier bandit: learned per-doc-kind tier policy, reward = gate-pass x tokens-saved.
- P4 `synx state <topic>`: supersession ledger -> current-truth card.

## Why P1 first
Without a real reward, all existing ML infra learns from a weak explicit-accept signal. Closing the loop (cited + gate-pass) is the cheapest change with the biggest multiplier — it makes everything downstream compound. This IS the Superverschnellerung thesis: the loop is what compounds, each session cheaper + sharper than the last.
