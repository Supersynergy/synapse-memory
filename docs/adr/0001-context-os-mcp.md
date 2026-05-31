# ADR 0001 — Context-OS as a cross-CLI MCP server

- Status: accepted
- Date: 2026-05-31

## Context

Agents on Claude Code, Codex and Gemini CLI all degrade with long/middle context
(context-rot, lost-in-the-middle). Synapse already had hybrid retrieval, a feedback loop
(`synapse-learn`) and a naive token-budgeted `agent_context`. We wanted one thing that any
of those CLIs can use to get always-best, budget-bounded, self-learning context.

## Decision

1. **Integration surface = MCP stdio.** All three CLIs speak MCP stdio JSON-RPC 2.0 and have
   an official `mcp add`. One `synapse-mcp` binary, three configs, wired by
   `scripts/install-ctxos.sh`. No per-CLI plugin code.
2. **Packing is verbatim deletion, not summarization.** For agent/code context, preserving
   exact file paths / error strings / numbers matters more than max compression ratio, and
   it removes hallucination + vendor lock. Implemented as a pure crate `synapse-pack`
   (tiers `full → signatures → fact-delta → one-line`, SimHash dedup, greedy budget knapsack,
   serial-position order). Pure ⇒ unit-testable without a daemon.
3. **Learning reuses `synapse-learn`.** `context_feedback` writes per-kind reward
   (`memory_type_reward`); `context_pack` reads `memory_type_bonus` and adds it to candidate
   scores. Self-contained loop, independent of the daemon, shared with `synx hybrid`.
4. **No new daemon protocol op.** The MCP server retrieves via the existing `Search` op and
   writes feedback directly to the sibling `*.learn.db`. Keeps the daemon unchanged.

## Consequences

- Works identically across CLIs and offline; deterministic, no LLM call in the hot path.
- Brain path is a convention (`~/.synapse/brain.db`), overridable via `--brain`/`SYNAPSE_BRAIN`.
  If a deployment puts the brain elsewhere, set the env var.
- `synapse-pack` is reusable by other surfaces (CLI, SDKs) later.
- Alternatives rejected: per-CLI native plugins (3× maintenance), LLM summarization
  (hallucination + cost + lock-in), opaque server-side compression (vendor lock, not inspectable).
