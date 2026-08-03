<!-- REPO-POLISH-AGENTS:START -->
# AGENTS.md

Synapse is a Rust-first developer tool focused on fast local workflows.

## Commands

- `test`: `just test`
- `lint`: `just lint`
- `fmt`: `just fmt`
- `check`: `just check`
- `setup`: `cargo fetch`
- `build`: `cargo build`

## Repo Rules

- Optimize for Time-to-First-Success: keep setup and verification commands obvious.
- Keep changes scoped to the domain being edited; avoid catch-all `utils`, `helpers`, and `misc` buckets.
- Preserve existing user changes in this repository. Do not run destructive git commands.
- Add or update tests when behavior changes.
- Put durable architecture rationale in `docs/adr/`.
<!-- REPO-POLISH-AGENTS:END -->

## Two-Layer Token-Saving Stack

This repo pairs with `agent-token-saver-skill-router` to cut prompt tokens on two layers:

| Layer | Tool | Path | Savings |
|---|---|---|---:|
| System-Prompt | agent-token-saver-skill-router | `~/.claude/skills/agent-token-saver-skill-router/SKILL.md` | 5-15k tok/req |
| Context-Pack | synapse-pack | `crates/synapse-pack/src/lib.rs` | 60-90 % vs naive |

See `CHEATSHEET.md` for the full pair-up reference.

### MCP Tools (synapse-mcp)

| Tool | Purpose |
|---|---|
| `context_pack` | Retrieve + pack minimal VERBATIM context (call FIRST) |
| `context_feedback` | Report used doc ids + verify-gate (closes self-learning loop) |
| `context_state` | Current-truth card for a topic (superseded-marked) |
| `context_remember` | Persist durable fact/decision |

### Funcmap

Function/flow map lives at `crates/synapse-pack/.grepgod/funcmap.md` (148 fns, 175 edges, 18 cross-repo). Rebuild after refactors:

```bash
cd ~/BASE/projects/synapse && grepgod --chain funcmap crates/synapse-pack crates/synapse-learn crates/synapse-mcp --lang rust
```

### Build-Out Targets (2026-07-22)

| Feature | Layer | Status |
|---|---|---|
| `pack_delta()` — incremental pack vs last `pack_id` | synapse-pack | ✅ shipped + 3 tests |
| Pack-Cache (LRU keyed by query hash) | synapse-mcp | ✅ shipped + 4 tests |
| `cache_stable_order` — prompt-cache-stable prefix ordering | synapse-pack | ✅ shipped + 2 tests |
| MCP-Tool-List-Truncation by query terms | synapse-mcp | ✅ shipped |
| `context_pack` wires `prev_pack_id` + `cache_stable_order` + `use_cache` | synapse-mcp | ✅ shipped |
| Imperative `instructions` (4 RULEs) for dumb models | synapse-mcp | ✅ shipped |
| Verify-Gate Degradation (budget shrinks without feedback) | synapse-mcp | ✅ shipped + 1 test |
| Skill-Preload-Hints in pack manifest | synapse-mcp → router | ✅ shipped + 3 tests |
| `has_context_trigger` — auto-pack trigger predicate | synapse-mcp | ✅ shipped + 1 test (Phase 3: wire to router) |
| `synapse-decay` crate (Ebbinghaus + interaction-graph) | new crate | ✅ scaffold + 8 tests (Phase 3: wire to recall) |
| `Kind::SessionSummary` + `Kind::CodebaseMap` | synapse-pack | ✅ shipped + 6 tests |
| `session_ingest` MCP tool (swarm/cmux event ingest) | synapse-mcp | ✅ shipped |
| `session_replay` MCP tool (mega-session reconstruction) | synapse-mcp | ✅ shipped |

### Agent Roles

| Agent | Task |
|---|---|
| Claude Code | synapse-pack implementation + tests |
| Codex CLI | synapse-mcp implementation + benchmarks |
| Windsurf | review + funcmap updates |
| Hermes | multi-agent orchestration for Phase 3 |
