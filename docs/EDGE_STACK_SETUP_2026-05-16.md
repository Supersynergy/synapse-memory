# Edge Stack Setup 2026-05-16

Purpose: keep Synapse as the local-first recall and freshness substrate while mining the newest memory/context tools for patterns, adapters, and fair benchmarks.

## Current Research Verdict

Top local-first memory candidate: OMEGA Memory. It is relevant because it ships a Python MCP server, SQLite/sqlite-vec storage, ONNX embeddings, contradiction/forgetting surfaces, and public LongMemEval claims. Treat those claims as vendor claims until reproduced locally.

Top context compression candidate: LeanCTX. It is relevant because it focuses on AST/file/shell compression and token-governed context delivery across coding tools.

Top Context7-style freshness candidates: Context/Neuledge, Docfork, GitMCP, DeepWiki, and Ref Tools. Synapse should keep the source-of-truth rule: code against resolved local package versions first; use remote docs tools only as fallback evidence.

Top temporal graph reference: Zep/Graphiti. Mine temporal validity and contradiction handling, but avoid Neo4j as a default dependency for Synapse local mode.

## Local Setup

- `ghmax`, `ghgrep`, `superscrape`, `smart-fetch`, and `batch-md-rs` are installed.
- OMEGA Memory is installed in an isolated venv at `.tools/omega-memory`.
- OMEGA model cache was upgraded to `bge-small-en-v1.5` under `~/.cache/omega/models/bge-small-en-v1.5-onnx`.
- OMEGA MCP was registered in `~/.codex/config.toml` as `omega-memory`, pointing at `.tools/omega-memory/bin/python -m omega.server.mcp_server`.
- LeanCTX `3.6.0` is installed via Cargo and registered as the Codex MCP server `lean-ctx`.
- `@neuledge/context` `1.1.0` is installed under `.tools/npm`; `react@latest` docs are installed locally and queried successfully.
- Docfork `2.2.1`, Context7 MCP `2.2.5`, GitMCP via `mcp-remote`, DeepWiki via `mcp-remote`, and Signet `0.116.3` are registered as Codex MCP servers.
- Signet is installed in `.tools/signet`, configured with an isolated agent workspace at `.tools/signet-agent`, and verified through daemon, HTTP recall, and MCP recall.
- Mem0 `2.0.2` is installed in `.tools/mem0`; local FAISS smoke testing works with mock embeddings and no cloud dependency.
- Cognee `1.0.9` is installed in `.tools/cognee`; session `remember`/`recall` works locally, while graph recall needs dataset permissions/config before adoption.
- Graphiti Core `0.29.0` is installed in `.tools/graphiti`; full temporal graph E2E remains gated on a configured graph backend and LLM/embedder.
- Letta `0.16.8` is installed in `.tools/letta`; an isolated Homebrew Postgres 17 + pgvector instance runs from `.tools/letta-pg` on port `5433`, and the Letta server is verified at `http://127.0.0.1:8283/v1/health/` via LaunchAgent `com.synapse.letta`.
- Synapse hook context now emits an `<edge_stack_context>` block for prompts about Omega, Context7, CTX/cortext, agent memory, recall, and version slippage.

## Verification Snapshot

- LeanCTX MCP handshake: 10 tools exposed; `ctx_read` and `ctx_shell` verified against Synapse files/tests.
- Context local docs: `context query react@latest useState` returned local React documentation.
- Docfork MCP: `search_docs` returned React `useState` documentation/code references.
- Context7 MCP: `resolve-library-id` returned React docs IDs.
- GitMCP remote: `search_next_js_documentation` returned current Next.js `revalidateTag` docs from `vercel/next.js`.
- DeepWiki remote: `read_wiki_structure` returned a Next.js repo wiki structure.
- Signet MCP: `memory_search` returned the stored smoke-test memory from the local daemon.
- Mem0 local: add/search returned one FAISS-backed smoke-test memory.
- Cognee local: `remember` stored a session memory and `recall(..., only_context=True)` returned it.
- Letta server: `/v1/health/` returns `{"version":"0.16.8","status":"ok"}`; Postgres contains default org/user and synced provider models.

## Integration Rule

Synapse remains the primary hot path. External tools are allowed into the runtime only after they beat or materially complement Synapse on a measured gate:

- recall: LongMemEval/LoCoMo/BEAM or a labeled project golden set
- freshness: resolved-version correctness and outdated API avoidance
- token saving: lower injected tokens at equal task success
- latency: p50/p95 under hook budgets
- local-first: no mandatory cloud, no heavyweight service dependency

## Next Gates

1. Run OMEGA `eval-retrieval` against a small shared labeled set.
2. Run Synapse LongMemEval-S full release mode after making the harness faster or resumable.
3. Add a Context7/Docfork/GitMCP source adapter only as a fallback behind Synapse `fresh-context`.
4. Benchmark LeanCTX compression on real Codex file-read/shell-output traces before adopting it as a dependency.
