<div align="center">

<img src="assets/banner.svg" alt="Synapse — one file. Your AI's entire memory." width="100%"/>

### Your AI finally remembers you. Everything. Forever.

One file on disk. Drop it in your project. Your AI picks up where you left off — last week, last month, last laptop.

[![Release](https://img.shields.io/github/v/tag/Supersynergy/synapsedb?label=release&color=blueviolet)](https://github.com/Supersynergy/synapsedb/releases)
[![CI](https://github.com/Supersynergy/synapsedb/actions/workflows/quality.yml/badge.svg)](https://github.com/Supersynergy/synapsedb/actions/workflows/quality.yml)
[![Coverage](https://codecov.io/gh/Supersynergy/synapsedb/branch/main/graph/badge.svg)](https://codecov.io/gh/Supersynergy/synapsedb)
[![MSRV](https://img.shields.io/badge/MSRV-1.95.0-orange)](rust-toolchain.toml)
[![License](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Stars](https://img.shields.io/github/stars/Supersynergy/synapsedb?style=flat&color=ffcc00)](https://github.com/Supersynergy/synapsedb/stargazers)
[![CRDT](https://img.shields.io/badge/CRDT-yrs-8a2be2)](crates/synapsedb-core/src/crdt.rs)
[![Ed25519](https://img.shields.io/badge/signed-Ed25519-22c55e)](crates/synapsedb-core/src/sign.rs)
[![MCP](https://img.shields.io/badge/MCP-5%20tools-0ea5e9)](crates/synapsedb-mcp/src/main.rs)
[![E2E](https://img.shields.io/badge/E2E-tested-16a34a)](bench/e2e_smoke.sh)
[![Self-Learning](https://img.shields.io/badge/self--learning-bandit%2Bheat-f59e0b)](crates/synapsedb-learn/)

</div>

---

## What it feels like

**Before Synapse**

> "Hey Claude, we talked about the database migration yesterday. Remember?"
>
> *Claude has no memory of yesterday.*
>
> You paste 40 lines of context. Again. Third time this week.

**After Synapse**

> "Hey Claude, where did we land on the migration?"
>
> *"You chose Postgres over SurrealDB on Tuesday — the BSL license was the blocker. Draft spec is in `/specs/db-migration.md`."*

---

## Three lines of actual code

```bash
cargo install --locked --git https://github.com/Supersynergy/synapsedb --tag v2.0.0 synapse synapsedb-daemon synapsedb-mcp
synapsedb-daemon -f ~/brain.db &
synapse put "postgres chosen over surrealdb" && synapse search "database decision?"
```

## Wire it into Claude Code in 30 seconds

Add to `~/.claude/settings.json`:

```json
{ "mcpServers": { "synapsedb": {
    "command": "synapsedb-mcp",
    "args": ["--sock", "/tmp/synapsedb.sock"]
  } } }
```

Restart Claude. Your agent now has `put`, `search` and `snap` as native tools. Cursor, Cline, Continue, Aider — same config.

---

## v2.0 — All features, all adapters

| Feature | Status | What it gives you |
|---------|--------|-------------------|
| **Ed25519 signing** | stable | tamper-evident memory — every entry cryptographically verified |
| **CRDT merge** (yrs) | stable | offline-first multi-writer collaboration, no server |
| **MCP server** (5 tools) | stable | `put / search / merge / timeline / verify` as native agent tools |
| **IVF sharding** (fastbloom) | stable | scales to 100 M entries without a cluster |
| **TCP/unix federation** | stable | peer-to-peer memory sync, no cloud |
| **Self-learning bandit** | stable | Thompson + heat + drift + consolidate — ranking improves as you use it |
| **Multi-extension format** | stable | `.syn / .synapse / .brainpack` — one reader, any source |
| **SQLCipher encryption** | feature-flagged | `--features encrypt` for at-rest AES-256 |

### Ecosystem adapters

Drop Synapse into any AI stack with one import:

| Adapter | Install | What it replaces |
|---------|---------|-----------------|
| **mem0-shim** | `pip install synapsedb-mem0` | mem0 drop-in — zero code changes |
| **mastra** | `npm i @synapsedb-ai/mastra` | MastraMemory via unix socket |
| **vercel-ai** | `npm i @synapsedb-ai/vercel-ai` | `createMemoryProvider` for `useChat` / `generateText` |
| **copilotkit** | `npm i @synapsedb-ai/copilotkit` | CopilotKit persistent context store |
| **langfuse** | `pip install synapsedb-langfuse` | `SynapseRetriever` with per-search span tracing |
| **promptfoo** | `pip install synapsedb-promptfoo` | eval provider + RAG benchmark YAML template |
| **browser-use** | `pip install synapsedb-browser-use` | `on_page_visit` hook — browse history in memory |

---

## 📡 NEW: Telepathy — cross-session memory for Claude Code

**Every parallel Claude Code session shares one live brain.**

Session A sees what Session B just edited. Session C knows the prompt D got two seconds ago. Kill a session, open a new one, pick up mid-thought. Nobody else ships this.

```bash
bash integrations/claude-code/telepathy/install.sh
```

A 22 MB daemon tails `~/.claude/projects/**/*.jsonl`, pushes compact events to Synapse, and a `SessionStart` + `UserPromptSubmit` hook re-injects recent cross-session activity.

| Metric | Value |
|---|---|
| End-to-end (jsonl → visible in other session) | ~3 s |
| `syn put` / `syn search` / hook | 50 / 22 / 47 ms |
| Daemon CPU idle | ~0 % |
| Full scan 8 676 jsonls | 58 ms |

→ Full docs: [`integrations/claude-code/telepathy/`](integrations/claude-code/telepathy/README.md)

---

## Top 20 real-world use cases

Drop-in memory that actually changes what you can build:

1. **Multi-session Claude swarm** — N Claude Code windows, one shared brain, live cross-session telepathy (new).
2. **Persistent pair-programmer** — agent remembers every architectural decision across weeks, no CLAUDE.md bloat.
3. **Cursor / Cline / Continue / Aider memory** — same brain, any IDE, switch freely mid-task.
4. **Offline-first team KB** — CRDT merge = multiple devs, multiple laptops, no server, conflicts auto-resolve.
5. **Tamper-evident decision log** — Ed25519 signatures on every entry, audit trail for compliance.
6. **mem0 drop-in replacement** — `pip install synapsedb-mem0`, zero code change, 10× faster, self-hosted.
7. **RAG without a vector DB** — single file replaces Pinecone/Chroma/Weaviate for up to 100 M entries.
8. **Agent hand-off** — orchestrator writes plan, workers read it, writers publish results, all through one brain.
9. **Long-running research agent** — 8-hour browser-use session accumulates findings, next session consumes them.
10. **PR-review memory** — reviewer agent keeps patterns across PRs; learns your codebase's conventions.
11. **Customer-support agent** — every ticket's context persists, support bot answers with full history.
12. **Self-improving prompts** — self-learning bandit reranks memories by what actually helped → better retrieval each session.
13. **Incident post-mortem corpus** — every outage + fix becomes searchable context for the next one.
14. **Spec-driven development** — living specs in Synapse; agents query them before editing code.
15. **Cross-repo refactor** — one session scans repo A, logs patterns, another session applies them in repo B.
16. **Sales / CRM knowledge graph** — lead notes, calls, decisions — agent writes a follow-up email with full context.
17. **Research-paper digest** — batch-ingest PDFs, agents cite sources across conversations.
18. **Laptop migration** — copy one `.syn` file, new machine has your AI's entire history.
19. **Air-gapped / EU-compliant deployment** — no cloud, no telemetry, no vendor lock-in, GDPR by default.
20. **LLM-provider portability** — swap Claude → GPT → local Llama, memory stays identical.

## Real-competitor bench (agent memory, 1 k docs, 200 q, 384-d)

Same corpus, same embeddings, run locally. Full script: [`bench/real_competitors.py`](bench/real_competitors.py) · narrative: [`bench/RESULTS-REAL-COMPETITORS.md`](bench/RESULTS-REAL-COMPETITORS.md).

| engine | insert ms | ms / query | size KB | carries |
|--------|----------:|-----------:|--------:|---------|
| FAISS flat (floor) | 0.20 | 0.010 | 1 500 | vector only |
| SQLite FTS5 (floor) | 2.03 | 0.012 | 140 | keyword only |
| **Synapse v2.0** | **67.0** | **0.023** | **1 290** | **BM25 + HNSW + KG + CRDT + sign + MCP + self-learn + sharding** |
| Chroma | 96.3 | 0.303 | 4 434 | vector only |
| LanceDB | 41.7 | 1.440 | 1 574 | vector only |

Synapse sits **2.3× off the theoretical FAISS floor** while carrying **eight capabilities** the floor doesn't.

## Head to head, numbers you can re-run

Every row below reproduces with one command: `bash bench/bench_20_usecases.sh`.
Full 50-usecase matrix + CatBoost-picked defaults: [`bench/RESULTS-V1.md`](bench/RESULTS-V1.md).

| op (10 k docs, M4 Max, p50) | Synapse | runner-up | gap |
|-----------------------------|--------:|----------:|----:|
| cold open | **0.69 ms** mmap | SQLite WAL ~ 7 ms | **10×** |
| BM25 query | **23 µs** | Meilisearch ~ 1.0 ms | **43×** |
| vector kNN k = 10 | **22 µs** | LanceDB ~ 50 µs | **2×** |
| CRDT merge, 200 ops | **0.59 ms** | Automerge baseline ~ 1.0 ms | **1.7×** |
| sign + verify manifest | **25 µs each** | `ed25519-dalek` stock | parity |
| pack → ship → verify → mmap | **12 ms end-to-end** | tar + cosign + sqlite ≈ 340 ms | **28×** |

## What you stop paying for

| What most teams run today | Synapse |
|---------------------------|---------|
| Pinecone / Qdrant + Redis + Postgres + a Python embedder + Docker | one binary + one file |
| Monthly bills, rate limits, vendor outages | free, offline, yours |
| 30-min copy-paste onboarding to share context with a teammate | `scp brain.brainpack you@laptop:~/` — done |
| "Sorry, I'm a new session, can you re-explain?" | every decision remembered |
| Homemade versioning in Notion, Slack threads, git commits | searchable timeline, one query |
| A sync server for multi-writer collaboration | merges just work, offline, no server |

You ship the same AI features with **90 % less infrastructure**.

## Why this works

- **It's one file.** Back it up with `cp`. Ship it with `scp`. Commit it with `git`. Inspect it with `diff`. You already know the tools.
- **It's signed.** When a teammate sends you a `brain.brainpack`, you know it came from them. Like a verified email, but for AI memory.
- **It's offline by default.** No cloud. No API key. No privacy policy to read. Your memories stay on your machine until you choose to ship one.
- **It's fast enough to feel instant.** Every search is quicker than a mouse click.
- **It's open forever.** The format is public domain. Even if this project disappeared tomorrow, every `.synx` file keeps working — in any language, on any machine.

## What it actually does

1. **Remembers every decision.** Your AI never forgets a choice, a name, a file path, a decision.
2. **Finds the right memory in a blink.** Full-text + meaning-based search, together, in the same query.
3. **Tracks how your thinking changes.** "We chose X last week. Then switched to Y on Tuesday." Both survive, in order.
4. **Keeps per-user, per-session, per-project memories separate.** Your agent knows who it's talking to.
5. **Syncs between your laptops without a server.** Two devices, same brain, no cloud.
6. **Ships memory like a PDF.** Sign it, send it, anyone can verify it.
7. **Dedupes automatically.** Say the same thing twice — stored once.
8. **Plugs into Claude Code, Cursor, Cline, Continue, Aider.** One JSON block, done.
9. **Free forever.** MIT for the code, public domain for the format.
10. **Works as the only piece of infra you need.** No database cluster. No embedding service. No sync server.

<details>
<summary><sub>Under the hood · for the engineers</sub></summary>

&nbsp;

Synapse is a single-file binary format (`.synx`) written in Rust. It fuses BM25 full-text (via Tantivy), HNSW + int8-quantized vector search, a temporal knowledge-graph layer, memory scopes (mem0 parity), Automerge CRDT sync, Ed25519-signed distribution, BLAKE3 content-addressed chunks, and a zero-copy `mmap` reader.

**Measured on M4 Max (Criterion-reproducible):**

- `23 µs` BM25 query (p50, 10 k docs)
- `22 µs` vector kNN k=10 (p50, 2 k × 64-d)
- `0.69 ms` cold open with `mmap`
- `0.59 ms` CRDT merge of 200 ops
- `25 µs` Ed25519 sign + verify
- `5.7 µs` RPC round-trip over AF_UNIX msgpack

**Architecture:**

```text
put → [ BM25 ∥ HNSW+PQ ∥ KG ] → fused rank → Ed25519-signed CRDT log → .synx
```

Reproduce: `cargo bench -p synapsedb-core --features full`. Full 50-usecase bench + CatBoost-picked defaults in [`bench/RESULTS-V1.md`](bench/RESULTS-V1.md). Recall eval (LoCoMo / LongMemEval) roadmap in [`docs/EVAL-HARNESS.md`](docs/EVAL-HARNESS.md).

</details>

## Compared to the field

**9 capabilities. 20 incumbents. Only Synapse ships them all in one file.**

<div align="center">

<a href="assets/matrix-full.svg"><img src="assets/matrix-full.svg" alt="Synapse vs the field — 20-tool capability matrix" width="100%"/></a>

<sub>Click for the full-size 2200×1500 version · searchable markdown table below.</sub>

</div>

<sup>Click the image for the full-size version. The short table below is the same data, text-searchable.</sup>

| Tool | BM25 · keyword | Vector · semantic | Graph · relations + time | Scopes · user / session | Sync · multi-writer | Signing · Ed25519 | One file | µs-RPC | License |
|------|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| **→ Synapse** | **✅** | **✅** | **✅** | **✅** | **✅** | **✅** | **✅** | **✅** | **MIT + CC0** |
| SQLite | ✅ | add-on | — | — | — | — | ✅ | — | Public |
| DuckDB | add-on | add-on | — | — | — | — | ✅ | — | MIT |
| SurrealDB | ✅ | ✅ | ✅ | — | — | — | — | — | ❌ BSL |
| PocketBase | ✅ | add-on | — | — | — | — | ✅ | — | MIT |
| Qdrant | — | ✅ | — | tag only | — | — | — | — | Apache |
| Meilisearch | ✅ | partial | — | — | — | — | — | — | MIT |
| LanceDB | ✅ | ✅ | — | — | — | — | partial | — | Apache |
| Chroma | — | ✅ | — | — | — | — | — | — | Apache |
| Weaviate | ✅ | ✅ | partial | tag only | — | — | — | — | BSD |
| Pinecone | — | ✅ | — | tag only | — | — | — | — | ❌ closed |
| memvid | ✅ | — | — | — | — | — | ✅ | — | MIT |
| mem0 | — | add-on | add-on | ✅ | — | — | — | — | Apache |
| Graphiti | — | add-on | ✅ | ✅ | — | — | — | — | Apache |
| cognee | — | add-on | ✅ | ✅ | — | — | — | — | Apache |
| Memori | — | — | — | ✅ | — | — | — | — | MIT |
| Zep | — | ✅ | partial | ✅ | — | — | — | — | Apache |
| Letta | — | ✅ | — | ✅ | — | — | — | — | Apache |
| Automerge | — | — | — | — | ✅ | — | — | — | MIT |
| RocksDB | — | — | — | — | — | — | ✅ | — | Apache |
| Parquet | — | — | — | — | — | — | ✅ | — | Apache |

<sup>**add-on** = needs an extension · **tag only** = namespace filter, no real scope concept · **partial** = available but limited · **BSL / closed** = not permissive OSS.</sup>

Full one-by-one breakdown: [`docs/COMPARISON-V1.md`](docs/COMPARISON-V1.md).

## 60-second tour of the idea

Your agent has a 200 K-token context and zero memory between sessions. The usual fix is a pipeline — Qdrant, Redis, a Python venv, a Docker compose, three SDKs — so a language model can remember what it did yesterday.

**The stack is the bug.** Synapse is one file, one binary, one process. Every capability above lives inside a `.synx` container you can `git commit`, `scp`, sign and hand to a teammate. That's it.

## Deeper dives

- [`docs/CLAUDE-CODE-MEMORY.md`](docs/CLAUDE-CODE-MEMORY.md) — five real agentic workflows
- [`docs/SYNX-FORMAT-V2.md`](docs/SYNX-FORMAT-V2.md) — binary format spec (CC0)
- [`docs/BRAINPACK-V2.md`](docs/BRAINPACK-V2.md) — distribution wrapper + signing
- [`docs/COMPARISON-V1.md`](docs/COMPARISON-V1.md) — 20-tool head-to-head
- [`bench/RESULTS-V1.md`](bench/RESULTS-V1.md) — 50-usecase benchmark, 5 categories
- [`docs/LICENSE-STRATEGY.md`](docs/LICENSE-STRATEGY.md) — MIT + CC0 split + monetisation paths
- [`docs/USECASES.md`](docs/USECASES.md) — 20 deployment recipes
- [`docs/STRATEGY.md`](docs/STRATEGY.md) — how Synapse tops each competitor on their home turf
- [`CHANGELOG.md`](CHANGELOG.md) — v0.2.0 → v1.0.0 history

## Ecosystem

- **Claude Code** — paste the MCP block above, restart, done
- **Any MCP agent** — Cursor, Cline, Continue, Aider share the same config
- **synapse-turbo** — [`tools/turbo`](tools/turbo) — Python daemon for sub-ms queries (3-tier cache: pre-computed + NumPy SIMD + ONNX). 1000x faster than CLI for agent hooks.
- **Node.js** — [`sdk/node`](sdk/node)
- **Python** — [`sdk/python/synapsedb_reader.py`](sdk/python/synapsedb_reader.py), stdlib + `zstandard` + `blake3`
- **DuckDB analytics** — `ATTACH 'brain.db' AS s (TYPE sqlite, READ_ONLY);`

## License

MIT for the code. CC0 for the format specification. The format outlives the repo — any language, any product, no asking.

---

<div align="center">

Built in Rust. Shipped in Germany. Open forever.

*by [Maxim Supersynergy](https://github.com/Supersynergy)*

</div>
