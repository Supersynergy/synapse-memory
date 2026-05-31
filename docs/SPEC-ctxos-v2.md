# Synapse Context-OS (ctxos) — Release SPEC v2

> 2026-05-31. Release-grade. Goal: one local-first MCP server that gives ANY
> Claude Code / Codex / Gemini CLI user always-best, token-budget-bounded,
> self-learning context. Zero cloud, zero vendor lock, one binary.

## 0. Why this wins (research-grounded)
- Agent context degrades long (context-rot) and middle (lost-in-the-middle, LongLLMLingua +21.4% RAG @ 1/4 tokens).
- For agents, VERBATIM deletion beats summarization: 0% hallucination, file-paths/error-strings/line-numbers survive exactly. (Morph Compact 98% verbatim 50-70%; OpenAI opaque 99.3% but vendor-locked + non-inspectable; summarization multi-session retention only 37%.)
- ACON: compression should be ADAPTIVE per segment — tool dumps aggressive, decisions/facts preserved.
- Prevention-first (FlashCompact): retrieve high-signal first → 3-4x fewer compactions. Synapse hybrid 8ms IS that prevention layer.
- => ctxos = local, deletion-based, adaptive-per-kind, serial-position, budget-knapsack. No LLM call. Deterministic. Identical across all 3 CLIs.

## 1. Universal integration surface = MCP stdio (CONFIRMED 2026)
All three speak MCP stdio JSON-RPC 2.0. Synapse already ships `synapse-mcp` (stdio).
- Claude Code: `mcpServers` in `.mcp.json` / `~/.claude.json`; or `claude mcp add`.
- Codex: `[mcp_servers.<name>]` in `~/.codex/config.toml` (command/args/env/cwd/timeouts/enabled_tools/approval_mode); or `codex mcp add <name> -- <cmd>`. Reads MCP `instructions` field (first 512 chars matter).
- Gemini CLI: `mcpServers` in `~/.gemini/settings.json` (command/args/env/cwd/timeout/trust/includeTools); transports stdio/SSE/HTTP.
=> ONE binary, THREE configs. Installer writes whichever CLI is present.

## 2. MCP tools exposed (the product)
1. `context_pack(task, budget_tokens=4000, kinds?, since?)` → packed STATE string + manifest{used_ids, dropped_ids, tokens_in/out, tiers}. THE core value.
2. `context_feedback(pack_id, used_ids[], gate="pass|fail|unknown")` → reward → synapse-learn. Closes the loop.
3. `context_state(topic)` → current-truth card: latest verified facts + supersession chain + open questions.
4. `context_remember(text, title?, kind="known-fact|decision")` → `synx put` writeback.
(plus existing synapse_search/put/find/merge/verify stay.)

MCP `instructions` (first 512 chars, Codex-critical): "Synapse Context-OS. Call context_pack FIRST for any task needing prior context — returns minimal verbatim STATE within a token budget, never narrative. After the turn, call context_feedback with the doc ids you actually used + whether your verify-gate passed, so retrieval self-improves. Use context_state to get current truth + what superseded what. Use context_remember to persist a durable fact/decision. All local, no cloud."

## 3. The PACK pipeline (crate `synapse-pack`, PURE, no IO)
Input: Vec<Candidate { id, text, score, kind, tokens }>, budget, query.
1. NORMALIZE tokens (tiktoken-ish heuristic: chars/4 + code-aware).
2. DEDUP near-identical (SimHash 64-bit + Hamming ≤3 → keep highest score). Never pay twice for one fact.
3. TIER per candidate (adaptive, ACON-style), deletion-based only:
   - T0 full: verbatim (known-fact, decision, error strings).
   - T1 signatures: headings + first/last line + every line with a number/path/identifier/code-fence.
   - T2 fact-delta: only lines matching fact-pattern (digits, =, ->, :, CAPS-terms, paths). caveman-for-retrieval.
   - T3 oneline: title + 1 highest-entropy line.
   Tier floor by kind: known-fact/decision ≥ T1; file/chat may go T3.
4. KNAPSACK select tier per candidate to maximize Σ(score × tier_value) s.t. Σtokens ≤ budget. Greedy-by-density + 0/1 refine.
5. ORDER serial-position: best first, 2nd-best LAST, weak in middle (lost-in-middle fix).
6. EMIT: header (STATE card: N facts, M dropped, budget used) + ordered blocks, each `[id|kind|tier] text`.
All pure functions → unit-testable without daemon. This is the IP.

## 4. Self-learning (reuse synapse-learn)
- pack_id logged with candidate ids + chosen tiers.
- context_feedback(used_ids, gate) → reward: used+gate=pass → +; retrieved-but-unused → −; gate=fail → dampen.
- Trains: (a) RRF weights (rrf_tune), (b) source-trust-priors per kind, (c) tier-policy bandit per kind (Beta arm: did this kind at this tier get used?), (d) budget-split.
- Reward = gate_pass_weight × used_ratio − token_cost_penalty. Quality gates cost (joint objective).
- Active-learning: only re-tune on high-disagreement queries.

## 5. "Always knows state" — supersession ledger
- Every context_remember tags `supersedes: <id>?` and `topic`.
- context_state(topic): hybrid filter kind∈{known-fact,decision} + topic → group → newest verified head + chain + lines flagged "open/TODO/unverified".
- Verbatim retention (no summarization) → solves 37% multi-session loss.

## 6. Release / distribution (usable for everyone, worldwide)
- Single static binary `synapse-ctxos` (musl/universal). No Python, no cloud.
- `synapse-ctxos install [--claude] [--codex] [--gemini] [--all]`: detects installed CLIs, backs up config, writes MCP server entry idempotently, prints verify steps.
- `synapse-ctxos doctor`: checks daemon reachable, configs valid, tools list.
- Fallback when no daemon: spawn embedded read-only brain from `~/.synapse/brain.db`.
- Install one-liner (docs): `curl -fsSL <release>/install.sh | sh` → fetches binary + runs `install --all`.
- README + ADR + LICENSE (CC0/MIT for the bridge). Demo gif. crates.io + GH release.

## 7. Acceptance (definition of done)
- [ ] `synapse-pack` unit tests green (dedup, tier, knapsack-budget-respected, serial-order, never-exceeds-budget).
- [ ] `synapse-ctxos` MCP handshake: tools/list returns 4 ctx tools; tools/call context_pack returns valid manifest within budget.
- [ ] `install` writes valid config for each of Claude/Codex/Gemini (parse-back test), idempotent, backup made.
- [ ] `doctor` green on this machine.
- [ ] Real end-to-end: pack a task at 2000-token budget, assert output ≤ budget, file-paths preserved verbatim, manifest ids resolve.
- [ ] feedback round-trips into synapse-learn (bandit prior moves).
- [ ] README + install.sh + ADR committed. `cargo clippy -D warnings` + `cargo nextest` green.

## 8. Build order (vertical slices, each verifiable)
- S1 `synapse-pack` pure crate + tests (TDD). ← core IP, no IO, fastest gate.
- S2 wire 4 MCP tools into a `synapse-ctxos` bin (reuse synapse-mcp plumbing + daemon client).
- S3 `install` + `doctor` subcommands (3-CLI config writers + parse-back tests).
- S4 feedback→learn wiring + tier-policy bandit.
- S5 context_state ledger.
- S6 release: README, install.sh, ADR, demo, version, CHANGELOG.
