# Query Decomposition

## Why
LongMemEval hard subset is dominated by composite queries ("what did X say
after meeting Y"). Recall on the full string under-performs because relevant
memories live behind two distinct semantic anchors.

## What
- `PipelineHooks::decompose(query) -> Vec<String>` in `synapse-core::sota_pipeline`.
- Default rule-based fallback splits on cue-words: `after | before | and then |
  while | vs | versus`. MlxExtractor / future LLM hook supplies real LLM-driven
  decomposition (langchain `MultiQueryRetriever` prompt port).
- `pipeline_recall()` runs `Store::recall` per sub-query, fuses via RRF on rank
  position with `k=60`, dedupes by `hit.id`.

## Tests
- `rule_hooks_decompose_after` confirms 2-way split on "after".
- Pipeline e2e covered indirectly by `evolve_supersedes_similar` flow.

## Source mining
- langchain.retrievers.multi_query.MultiQueryRetriever
- llamaindex.query_engine.SubQuestionQueryEngine
