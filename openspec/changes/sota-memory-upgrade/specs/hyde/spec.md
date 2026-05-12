# HyDE — Hypothetical Document Embeddings

## Why
Sparse / under-specified queries return <3 hits in 12 % of LongMemEval-S.
HyDE (Gao et al. 2022) generates a plausible answer first, embeds THAT, and
re-searches — converts queries that miss into queries that hit.

## What
- `PipelineHooks::hyde(query) -> String` in `synapse-core::sota_pipeline`.
- Default: deterministic prompt-template fallback (echo + expansion).
- Mlx impl: smollm2-1.7B with HyDE prompt template.
- `pipeline_recall()` triggers HyDE re-search when initial fused candidate
  count < `hyde_threshold` (default 3). Extra hits merged via the same RRF.

## Tests
- `rule_hooks_hyde_nonempty` ensures fallback always produces ≥10-char output.

## Source mining
- Gao et al. 2022, "Precise Zero-Shot Dense Retrieval without Relevance Labels".
- llamaindex `HyDEQueryTransform`.
