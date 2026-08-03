# synapse — AI Cheatsheet

**Stack:** Rust workspace (synapse-pack + synapse-learn + synapse-mcp + synapse-decay). Local-first Context-OS.
**VCS:** Git + jj + Scalar + Dolt (Tier S, AI-Dev 7,0).
**Funcmap:** 197 defs, 309 edges (35 cross-repo). Hotspots: `get` (main.rs:59), `pack` (lib.rs:265), `default` (lib.rs:179), `cand` (lib.rs:624), `estimate_tokens` (lib.rs:200), `daemon_call` (main.rs:505).

## Commands
- `just test` — cargo test (alle crates)
- `just lint` — clippy -D warnings
- `just fmt` — cargo fmt
- `just check` — cargo check
- `just bench` — cargo bench (delta_bench)
- `cargo test -p synapse-mcp` — nur MCP-Tests
- `cargo test -p synapse-pack` — nur Pack-Tests

## Routing
| Intent | Skill |
|---|---|
| Pack-Logic ändern | `ponytail` + `verification-loop` |
| MCP-Tool hinzufügen | `clear-thought` + `metareview` |
| Token-Savings-Benchmark | `agent-token-saver` + `speedtuning` |
| Decay-Crate erweitern | `superml` (Ebbinghaus-Model) |
| Funcmap-Rebuild | `grepgod --chain funcmap crates/synapse-pack crates/synapse-learn crates/synapse-mcp crates/synapse-decay` |
| Cross-Repo-Review | `metareview` + `three-brain` (Codex) |

## superfast-Oracle-Beispiele
- `cargo test -p synapse-pack` exits 0
- `cargo test -p synapse-mcp -- --nocapture` exits 0
- `just bench delta_bench` → p50 < 200µs
- `cargo nextest run -p synapse-pack` → alle grün

## Top-Bottleneck (TOC)
`crates/synapse-mcp/src/main.rs:59` — 40× `get` = MCP-Tool-Dispatch, Hauptpfad für alle Anfragen.

## Phase-3-Open
- `has_context_trigger` → Router wiring
- `synapse-decay` → Recall-Integration
