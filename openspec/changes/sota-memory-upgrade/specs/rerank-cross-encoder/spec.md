# Capability: rerank-cross-encoder

## ADDED Requirements

### Requirement: Reranker trait
`synapse-rerank::Reranker` SHALL expose `rerank(query: &str, candidates: Vec<Hit>) -> Vec<Hit>` with stable ordering for ties.

#### Scenario: IdentityReranker is no-op
- **GIVEN** input candidates `[a, b, c]`
- **WHEN** `IdentityReranker::default().rerank(q, input)` is called
- **THEN** output equals input (same order, same items)

### Requirement: ONNX cross-encoder default model
When the `onnx` cargo feature is enabled, `OnnxCrossEncoder::default()` SHALL load `JINA-rerank-v2-base-multilingual` via fastembed v5.

#### Scenario: Default model identifier
- **GIVEN** `OnnxCrossEncoder::default()`
- **WHEN** the model id is queried
- **THEN** it SHALL contain the substring `"jina"` and `"rerank-v2"`

### Requirement: Top-k cap
The cross-encoder SHALL only rerank top-N candidates (default N=20) to bound latency.

#### Scenario: Beyond cap untouched
- **GIVEN** 100 input candidates and `top_n = 20`
- **WHEN** rerank runs
- **THEN** positions 21..100 are returned in original order; positions 1..20 are reranked
