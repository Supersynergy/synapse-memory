# synapse-space ROADMAP

Hierarchy: `Space → Wing → Room → Drawer`

---

## Done (v0.1)

- `Space::open` / `Wing` / `Room` / `Drawer::put` verbatim store (FTS5 + optional vec)
- `Space::search` hybrid (BM25 + vec) and FTS-only paths
- 6 MCP tools: `space_create`, `wing_add`, `room_add`, `drawer_put`, `space_search`, `space_wake_up`
- Integration test: 3 drawers, search, assert top hit
- ADR-001: no Mojo

---

## Remaining MCP tools (23 of 29)

### P0 — Recall-critical (highest recall-impact-per-LOC)

| Tool | Description |
|------|-------------|
| `drawer_list` | list all drawers in a Room (paged, sorted by recency) |
| `drawer_show` | fetch one drawer by id or uri |
| `drawer_delete` | soft-delete a drawer (tombstone in registry) |
| `space_sweep` | one verbatim drawer per user/assistant message, idempotent by msg hash |
| `wing_search` | search scoped to one wing only |

### P1 — Knowledge graph ops

| Tool | Description |
|------|-------------|
| `kg_add_entity` | add named entity (person/project/concept) to SQLite KG table |
| `kg_add_relation` | add typed edge between two entities with temporal validity window |
| `kg_invalidate` | mark entity or relation as invalidated after a date |
| `kg_timeline` | list entities/relations active at a given date |
| `kg_query` | FTS + relation traversal: "what did alice say about X after 2025?" |

### P1 — Agent diary

| Tool | Description |
|------|-------------|
| `diary_create` | create a named diary (auto-wing: "diary") |
| `diary_append` | append timestamped entry to a diary |
| `diary_list` | list diary entries with date filter |

### P1 — Cross-wing nav

| Tool | Description |
|------|-------------|
| `wing_list` | list all wings in a Space |
| `room_list` | list rooms in a wing |
| `space_graph` | visual adjacency summary (wings → rooms → drawer counts) |

### P2 — Retrieval quality

| Tool | Description |
|------|-------------|
| `space_recall` | hybrid retrieval v4/v5: keyword boost + temporal proximity + preference pattern |
| `space_wake_up_full` | load full context summary for a new session (expand current skeleton) |
| `drawer_evolve` | compact N drawers in a room into one summary drawer (HippoRAG-2 style) |

### P2 — Mining flows

| Tool | Description |
|------|-------------|
| `mine_project` | index all files in a project directory into a wing |
| `mine_session` | parse Claude Code session JSONL and sweep into diary |
| `mine_transcript` | parse arbitrary conversation text and sweep drawers |
| `space_export` | dump all drawers to JSONL for backup or migration |
| `space_import` | bulk-load from exported JSONL |

### P2 — Claude Code hooks

| Tool | Description |
|------|-------------|
| `hook_autosave` | periodic drawer put every N tool calls |
| `hook_precompact` | force sweep before /compact to preserve context |

---

## Out of Scope (deliberate)

| What | Why |
|------|-----|
| Python runtime / pyo3 surface | pure Rust binary; Python adapter is a separate package |
| ChromaDB or external vector deps | synapse-engine native vec only |
| External embedder service | MLX-Metal direct via synapse-metal |
| pip distribution | single Rust binary via cargo + brew tap |
| Mojo | see ADR-001 |
| Cloud sync / telemetry | local-first invariant |

---

## Priority legend

- **P0**: ship next sprint — blocks recall quality
- **P1**: high value, medium LOC
- **P2**: nice-to-have or long tail; tackle after P0+P1 green
