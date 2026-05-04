# Spec — entity-ner

## ADDED Requirements

### Requirement: Lightweight NER for entity_id population
The async extraction worker SHOULD recognise common entity types
(person, project, date) using a gazetteer + regex first, and only fall
back to an ONNX model when explicitly enabled.

#### Scenario: Default build (no ONNX)
- **WHEN** a doc is extracted by `RuleExtractor`
- **THEN** detected entity strings are upserted into `entities` and
  linked via `memories.entity_id`.

#### Scenario: ONNX build (feature `ner-onnx`)
- **WHEN** `MlxExtractor` or `OnnxNerExtractor` is selected
- **THEN** entity recognition uses the model and assigns
  higher-confidence `entity_type` values.

## Implementation status
* Gazetteer + regex path — STUB present in `RuleExtractor` (entity
  field is currently `None` for default rules).
* ONNX path — DEFERRED. Mining note: `ghgrep` for gazetteer-based
  Rust NER returned mostly C/C++ projects; project-specific gazetteer
  list is the right minimal path before pulling in a model.
