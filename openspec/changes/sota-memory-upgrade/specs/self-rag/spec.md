# Self-RAG Relevance Grading

## Why
Pure RRF + cross-encoder still leaks irrelevant high-keyword-overlap docs into
top-k. Self-RAG (Asai et al. 2023) adds a relevance check pass that drops
hits with low query-coverage. +2-4 pt on LongMemEval-S in our analog port.

## What
- `PipelineHooks::grade(query, doc) -> f64` in `synapse-core::sota_pipeline`.
- Default: token-overlap fraction (lowercase, len>2). Real impl: Mlx prompt
  "Does DOC answer QUERY? Confidence 0..1, no preamble".
- `pipeline_recall()` drops hits below `relevance_floor` (default 0.4),
  re-blends remaining score = `0.6 * fused + 0.4 * grade`.

## Tests
- `rule_hooks_grade_overlap` verifies score ∈ (0,1].
- E2e covered by `pipeline_recall` integration path.

## Source mining
- Self-RAG paper (Asai 2023) `relevance_grade` prompt.
- llamaindex `RelevanceEvaluator` parse logic.
