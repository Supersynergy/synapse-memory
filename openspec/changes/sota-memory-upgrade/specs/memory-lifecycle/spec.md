# Spec — memory-lifecycle

## ADDED Requirements

### Requirement: Heat decay during recall
Recall SHALL down-weight candidate scores by recency when `params.heat`
is true. The decay function uses 0.97 ^ age_days clamped to 0.3 minimum,
which gives a ~30-day half-life and protects ancient-but-relevant memories
from being ranked at zero.

#### Scenario: Recent memory ranks above identical old memory
- **GIVEN** two memories with the same fused-RRF score, one updated today
  and one updated 90 days ago
- **WHEN** `recall()` runs with `heat: true`
- **THEN** the recent memory ranks first.

### Requirement: Evolve and compact (planned)
The lifecycle daemon MUST support:
* **evolve** — when a new memory is 0.55–0.95 cosine-similar to an existing
  one, append the new text to the existing row instead of inserting a
  duplicate.
* **compact** — periodically Jaccard-cluster near-duplicates and ask the
  configured `Extractor` to summarise each cluster into one canonical
  memory; supersede the cluster members via `memory_edges`.

## Implementation status
* Heat decay — IMPLEMENTED in `Store::recall`.
* Evolve / compact — DESIGN ONLY. Will extend
  `synapse-learn::consolidate` with an `Extractor` callback and run via
  launchd nightly.
