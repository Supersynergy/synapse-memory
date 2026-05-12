# Capability: memory-typed

## ADDED Requirements

### Requirement: Typed memory rows
The system SHALL persist memories with explicit `memory_type` ∈ {Fact, Preference, Decision, Lesson, Episodic, Raw}.

#### Scenario: Default weights applied
- **GIVEN** a memory inserted with `memory_type = Fact`
- **WHEN** `MemoryType::default_weight()` is queried
- **THEN** the value SHALL be `1.20`

#### Scenario: All six types weighted distinctly
- **GIVEN** the six MemoryType variants
- **WHEN** their default_weight values are collected
- **THEN** the set SHALL equal `{1.20, 1.15, 1.10, 1.05, 1.00, 0.95}` (Fact, Pref, Decision, Lesson, Raw, Episodic)

### Requirement: Supersession via `superseded_by`
A memory SHALL be marked superseded by setting `superseded_by` to the id of its replacement; the row is NOT deleted.

#### Scenario: Supersede chain queryable
- **GIVEN** memory A superseded by memory B
- **WHEN** `Store::supersede(A, B)` is called
- **THEN** A.superseded_by = B.id AND A still exists in the table

### Requirement: Idempotent migration
`sota_migrate(conn)` SHALL be safe to call multiple times on the same DB without error.

#### Scenario: Double-migrate is no-op
- **GIVEN** a fresh Store
- **WHEN** `sota_migrate()` is called twice
- **THEN** both calls return Ok and schema is unchanged after the second call

### Requirement: Memory edges with composite PK
`memory_edges` SHALL use composite PK `(src_id, dst_id, edge_type)` allowing multiple edge types between same pair.
