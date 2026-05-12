# Capability: extraction-async

## ADDED Requirements

### Requirement: Extractor trait
`synapse-extract::Extractor` SHALL expose `extract(doc_text: &str) -> ExtractionResult` returning entities + typed memories + edges + confidence.

#### Scenario: RuleExtractor zero-dependency baseline
- **GIVEN** doc text containing "Maxim prefers Rust over Go"
- **WHEN** `RuleExtractor::default().extract(text)` runs
- **THEN** result.entities contains "Maxim", "Rust", "Go"
- **AND** result.memories contains at least one `MemoryType::Preference`

### Requirement: Async extraction queue
Inserting a doc SHALL enqueue it for extraction (FIFO by `enqueued_ts`); the queue worker SHALL process in batches.

#### Scenario: Enqueue then pop batch
- **GIVEN** 3 docs ingested via `ingest_and_extract`
- **WHEN** `pop_extraction_batch(2)` is called
- **THEN** 2 doc rows are returned with `status` flipped to `'in_progress'`

#### Scenario: Failed extraction increments attempts
- **GIVEN** an extractor that returns Err
- **WHEN** `run_once` processes a queue item
- **THEN** the row's `attempts` += 1 AND `last_error` is populated AND `status = 'pending'`

### Requirement: MLX extractor stays off critical path
`MlxExtractor` SHALL invoke smollm2-1.7B-Instruct-4bit via subprocess (not embedded), so retrieval latency is unaffected by model loading.

#### Scenario: MlxExtractor disabled by default
- **GIVEN** default crate features
- **WHEN** `MlxExtractor::new()` is called
- **THEN** it returns Err unless the `mlx` feature is enabled

### Requirement: Idempotent entity upsert
`upsert_entity(canonical_name, type, aliases)` SHALL return the existing id if `canonical_name` matches (UNIQUE constraint), else insert.

#### Scenario: Same name twice → same id
- **GIVEN** `upsert_entity("Rust", "Language", [])` returns id=1
- **WHEN** called again with same name
- **THEN** it returns id=1 (no new row)
