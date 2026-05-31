# Synapse Context-OS — usage

A local-first MCP server that gives any agent CLI (Claude Code, Codex, Gemini CLI, or
anything that speaks MCP stdio) **always-best, token-budget-bounded, self-learning** context.

- **Verbatim** (deletion-based) — file paths, error strings, line numbers, numbers survive
  character-for-character. Zero hallucination, no vendor lock, no cloud.
- **Budget-bounded** — you ask for N tokens, you never get more.
- **Self-learning** — feedback on what you used lifts the right kinds of knowledge next time.

## Install (worldwide one-liner)

```bash
curl -fsSL https://raw.githubusercontent.com/supersynergy/synapse/main/scripts/install.sh | sh
```

This downloads a sha256-verified prebuilt `synapse-mcp` for your platform, installs it to
`~/.local/bin`, re-signs on macOS, then registers it into every agent CLI you have. If no
prebuilt exists it falls back to `cargo install --git` and then to a local source build.

Prebuilt targets (from the `release-ctxos` CI): `aarch64-apple-darwin`, `x86_64-apple-darwin`,
`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`. Other platforms build from source.

Rust users can also: `cargo install --git https://github.com/supersynergy/synapse synapse-mcp`.

### From a checkout

Requires the Synapse daemon (`synapsed`) running and `synapse-mcp` built
(`cargo build --release -p synapse-mcp`, or installed at `~/.local/bin/synapse-mcp`).

```bash
# one command — detects every CLI you have and registers the MCP server
sh scripts/install-ctxos.sh install --all

# or target specific CLIs
sh scripts/install-ctxos.sh install --claude --codex
sh scripts/install-ctxos.sh doctor      # verify daemon + binary + registrations
sh scripts/install-ctxos.sh uninstall   # remove from all CLIs
```

Under the hood it uses each CLI's official command:

| CLI | Command written |
|-----|-----------------|
| Claude Code | `claude mcp add -s user synapse -- <bin>` |
| Codex | `codex mcp add synapse -- <bin>` (`~/.codex/config.toml`) |
| Gemini CLI | `gemini mcp add -s user --trust synapse <bin>` (`~/.gemini/settings.json`) |

Env knobs: `SYNAPSE_MCP_BIN` (binary path), `SYNAPSE_SOCK` (daemon socket, default
`/tmp/synapse.sock`), `SYNAPSE_BRAIN` (brain DB; its sibling `*.learn.db` holds reward
tables, default `~/.synapse/brain.db`), `SCOPE` (`user`|`project`).

> Gemini disables MCP servers in **untrusted folders**. Trust the folder in Gemini, or run
> from a trusted one, to use the tools there.

## Tools

### `context_pack(query, budget_tokens=4000, k=24, kinds?)`
Retrieve + pack the minimal verbatim STATE for a task. Returns:
```json
{ "pack_id": "pk_…",
  "context": "STATE [11 facts · 3 dropped · 0 deduped · 525/600 tok · 59% saved] …",
  "manifest": { "used_ids": [...], "dropped_ids": [...], "deduped_ids": [...],
                "used_tokens": 525, "budget_tokens": 600, "savings_pct": 59.0,
                "blocks": [ {"id":…,"kind":"known-fact","tier":"Signatures","tokens":…} ] } }
```
`kinds` optionally filters to e.g. `["known-fact","decision"]`.

### `context_state(topic)`
Current-truth card: latest verified facts + decisions for a topic, newest-first,
superseded docs dropped, open/TODO/unverified lines flagged `⚠open`.

### `context_feedback(used_ids, gate="pass|fail|unknown", pack_id?)`
Report which doc ids you actually used and whether your verify-gate passed. `pass` rewards
the used kinds, `fail` dampens them. This moves the per-kind ranking bonus that
`context_pack` applies, so retrieval improves over time. Best wired into a turn-end hook.

### `context_remember(text, kind="known-fact|decision", title?, topic?, supersedes?)`
Persist a durable fact/decision (embedded + searchable). `supersedes` links an older doc id
so `context_state` can show what replaced what.

## How packing works (pure, deterministic, no LLM call)

1. **Retrieve** — hybrid RRF over the brain (~8 ms), top-`k` candidates.
2. **Learned prior** — add each kind's reward bonus (from feedback) to its score.
3. **Dedup** — SimHash-64, drop near-duplicates (Hamming ≤ 3).
4. **Tier (adaptive, deletion-based)** per candidate, by remaining budget:
   `full → signatures → fact-delta → one-line`. Known-facts/decisions never fall below
   *signatures*; file/chat dumps may go to *one-line*.
5. **Knapsack** — greedy multiple-choice: richest tier that fits, hard budget cap.
6. **Order** — best first, second-best last (mitigates lost-in-the-middle).

Research basis: verbatim compaction (Morph), token-pruning trade-offs (LongLLMLingua),
adaptive per-segment compression (ACON), serial-position effect. See `docs/SPEC-ctxos-v2.md`.

## Self-learning loop

```
context_pack ──► agent uses some doc ids ──► context_feedback(used_ids, gate=pass)
     ▲                                                   │
     └──────── memory_type_bonus per kind ◄── memory_type_reward (wins/losses) ◄┘
```
Stored in `~/.synapse/brain.learn.db` (`memory_type_reward`, `learn_bandit`). The CLI's
`synx hybrid` ranking reads the same table, so feedback compounds across CLI and MCP.
