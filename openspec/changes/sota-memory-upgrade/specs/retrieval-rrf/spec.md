# Capability: retrieval-rrf

## ADDED Requirements

### Requirement: Weighted typed RRF
`rrf_typed(ranked_lists, type_weights, k)` SHALL fuse N ranked lists using Reciprocal Rank Fusion with per-memory-type weight multipliers.

#### Scenario: k=60 default
- **GIVEN** `rrf_typed` invoked with no override
- **WHEN** computing scores
- **THEN** the constant `k` SHALL be `60` (Haystack / MongoDB / LlamaIndex industry default)

#### Scenario: Type weight multiplies RRF contribution
- **GIVEN** a hit of `MemoryType::Fact` (weight 1.20) at rank 1 in list A
- **AND** a hit of `MemoryType::Episodic` (weight 0.95) at rank 1 in list A
- **WHEN** `rrf_typed` runs with both lists single-source
- **THEN** the Fact hit final score SHALL exceed the Episodic hit score by ratio ≈ 1.20 / 0.95

### Requirement: RecallParams API
`Store::recall(params: RecallParams) -> Vec<Hit>` SHALL accept query, top_k, optional period filter, optional type filter, optional reranker.

#### Scenario: Top-k bound respected
- **GIVEN** RecallParams { top_k: 5 }
- **WHEN** recall returns
- **THEN** result.len() ≤ 5

#### Scenario: Period filter excludes out-of-range memories
- **GIVEN** memories from 2026-04-01 and 2026-04-29
- **AND** RecallParams.period = (2026-04-25, now)
- **WHEN** recall runs
- **THEN** only the 2026-04-29 memory is in results
