# Synapse-Pack × Agent-Token-Saver — Cheatsheet

**Stand:** 2026-07-22 · **Funcmap:** `crates/synapse-pack/.grepgod/funcmap.md` (148 fns, 175 edges, 18 cross-repo)

## Two-Layer Token-Saving Stack

```
User-Request
  ↓
[agent-token-saver]  System-Prompt-Ebene    → 0-1 Skills geladen (statt 50)  → -10k tokens
  ↓
[synapse-pack]       Context-Pack-Ebene     → verbatim Deletion-Tiers        → -70 % bei Inkrement
  ↓
[LLM]
```

## Layer 1 — agent-token-saver-skill-router

**Datei:** `~/.claude/skills/agent-token-saver-skill-router/SKILL.md` (318 Zeilen)

| Konzept | Wert |
|---|---|
| Default Load Budget | 0-1 Skills (auto route) |
| Max Load Budget | 10 (explicit only) |
| Cache-Index | `~/.cache/agent-token-saver/skills-index.json` |
| State-Dir | `~/.local/state/agent-skill-router/` |
| Router-Hook | outside model context (shell) |
| Combo-Partner | just-in-time-skill-router, token-budget-advisor |

**Regel:** Sobald die nächste konkrete Aktion klar ist, keine weiteren Skills laden.

## Layer 2 — synapse-pack

**Datei:** `/Users/master/BASE/projects/synapse/crates/synapse-pack/src/lib.rs` (746 Zeilen, 38 funcs)

### Pipeline (4 Stufen)

```
candidates → 1.trust-adjust → 2.SimHash-dedup → 3.knapsack-over-tiers → 4.serial-position
```

### Kind → Floor-Tier (ACON)

| Kind | Trust-Prior | Floor-Tier | Bedeutung |
|---|---:|---|---|
| `KnownFact` | +0,05 | `Signatures` | Curated verified fact — nie unter T2 |
| `Decision` | +0,03 | `Signatures` | ADR — nie unter T2 |
| `File` | 0,00 | `OneLine` | Source-Excerpt — darf auf T3 |
| `Chat` | -0,01 | `OneLine` | Session-Transcript — darf auf T3 |
| `Other` | 0,00 | `OneLine` | Default |

### Tier-Ladder (reich → mager)

| Tier | Token-Reduktion | Behält |
|---|---:|---|
| `Full` (T0) | 0 % | alles, verbatim |
| `Signatures` (T1) | ~40 % | headings, code-fences, fact-lines, first/last |
| `FactDelta` (T2) | ~70 % | nur Zeilen mit Zahlen/`=`/`->`/Pfaden/CAPS |
| `OneLine` (T3) | ~90 % | höchst-Signal-Zeile |

### Garantien

- `result.used_tokens <= options.budget_tokens` (hard cap)
- Verbatim — jede Output-Zeile ist Zeichen-für-Zeichen aus Input
- Deterministisch — keine LLM-Calls, keine Zufälligkeit
- Min-One — kleinster Budget gibt trotzdem Top-Fakt zurück

### Token-Savings-Matrix (gemessen an Test-Corpus)

| Szenario | Naive | synapse-pack | Savings |
|---|---:|---:|---:|
| 10 KnownFacts @ 4000 tok budget | 12.500 | 3.950 | 68 % |
| 50 Chat-Dumps @ 2000 tok budget | 45.000 | 1.980 | 96 % |
| 5 Decisions @ 1000 tok budget | 3.200 | 980 | 69 % |
| 1 File @ 200 tok budget | 1.800 | 180 | 90 % |

## Geplante Erweiterungen (dieses Build-Out)

| Feature | Layer | Token-Savings | Aufwand |
|---|---|---:|---|
| **Delta-Pack** | synapse-pack | -70 % bei Inkrement-Loops | 1d |
| **Pack-Cache (LRU)** | synapse-mcp | -100 % bei Repeat-Query | 0,5d |
| **Prompt-Cache-stable Render** | synapse-pack | -20 % bei Claude (1h-Cache) | 0,5d |
| **MCP-Tool-List-Truncation** | synapse-mcp | -500-2000 tok/Request | 0,5d |
| **Skill-Preload-Hints** | synapse-mcp → router | -80 % bei Router-Hit | 1d |
| **synapse-decay (Ebbinghaus)** | neuer Crate | bessere Recall-Qualität | 3d |

## Funcmap-Hotspots (Top 10)

| fan-in | function | file:line |
|---:|---|---|
| 9 | `estimate_tokens` | `synapse-pack/src/lib.rs:167` |
| 9 | `pack` | `synapse-pack/src/lib.rs:232` |
| 7 | `daemon_call` | `synapse-mcp/src/main.rs:384` |
| 7 | `cand` | `synapse-pack/src/lib.rs:521` |
| 6 | `agent_scope` | `synapse-mcp/src/main.rs:417` |
| 5 | `doc_id` | `synapse-mcp/src/main.rs:590` |
| 5 | `doc_text` | `synapse-mcp/src/main.rs:594` |
| 4 | `query_terms` | `synapse-mcp/src/main.rs:469` |
| 4 | `savings_pct` | `synapse-pack/src/lib.rs:130` |
| 4 | `kind_tag` | `synapse-pack/src/lib.rs:358` |

## Cross-Repo-Kanten (Re-Wiring-Surface)

| caller (synapse-mcp) | callee (synapse-pack) |
|---|---|
| `compact_hit` | `estimate_tokens` |
| `agent_get_observations` | `estimate_tokens` |
| `agent_context` | `estimate_tokens` |
| `context_pack` | `kind_tag`, `pack`, `render`, `savings_pct` |
| `context_feedback` | `kind_tag` |
| `context_state` | `kind_tag` |
| `hit_kind` | `from_meta` |
| `market_tool_call` | (synapse-learn) `open` |
| `noise_features_and_logistic_apply` | `default` |

## CLI-Quick-Commands

```bash
# Funcmap neu bauen
cd ~/BASE/projects/synapse && grepgod --chain funcmap crates/synapse-pack crates/synapse-learn crates/synapse-mcp --lang rust

# Tests
cd ~/BASE/projects/synapse && cargo test -p synapse-pack

# Benchmark
cd ~/BASE/projects/synapse && cargo bench -p synapse-pack

# MCP-Server starten
synx daemon &
cargo run -p synapse-mcp

# Smoke-Test Context-Pack
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"context_pack","arguments":{"query":"neocortex mcp","budget_tokens":2000}}}' | cargo run -p synapse-mcp
```

## Anti-Patterns (nicht tun)

- ❌ Summarization-LLM-Call im Pack (bricht Verbatim-Garantie)
- ❌ Skill-Voll-Katalog in System-Prompt laden (bricht Router)
- ❌ Pack ohne `header_reserve` (STATE-Card fehlt)
- ❌ Tier unter Floor drücken (bricht KnownFact/Decision-Trust)
- ❌ SimHash-Schwellwert > 3 (falsche Dedup-Positives)
