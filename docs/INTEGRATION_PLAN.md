# Synapse Integration Plan

## 1. Positioning

Synapse is a single-file agent-memory backend (one `.syn` SQLite file) that ships FTS5 full-text search, sqlite-vec ANN retrieval, MCP server, Ed25519 signing, CRDT merge, sharding, and federation — no separate vector DB, no cloud dependency, no infra overhead. A running agent embeds it as a library or connects via MCP in under 60 seconds.

The competitive gap: mem0 requires a cloud account or self-hosted stack of multiple services; Qdrant requires a separate server process; Pinecone is SaaS-only. Synapse replaces all three with a single Rust binary and a file. For OSS agents, local-first tooling, and regulated/air-gapped environments this is the default choice, not a trade-off.

---

## 2. Tier-1 Targets — Build NOW

| Target | Stars | Adapter | Status | Owner | ETA |
|---|---|---|---|---|---|
| mem0 | 53k | `synapse-mem0` (PyPI) | 🟡 in progress | — | v0.3.1 |
| mastra | 23k | `@synapse/mastra` (npm) | todo | — | v0.4 |
| vercel/ai | 24k | `useMemory` provider | todo | — | v0.4 |

### synapse-mem0 (PyPI)

Drop-in shim that mirrors `mem0.Memory` so existing `from mem0 import Memory` code works unchanged.

**Key methods:**

```python
mem = Memory(provider="synapse", path="./agent.syn")
mem.add(messages, user_id=...)       # store episodic memory
mem.search(query, user_id=..., limit=10)  # FTS5 + vec hybrid
mem.get_all(user_id=...)             # full recall
mem.delete(memory_id)
mem.reset()
```

**Repo structure:** `adapters/python/synapse-mem0/` — single `memory.py` wrapping the MCP client + sqlite bindings.

**Install:** `pip install synapse-mem0`

---

### @synapse/mastra (npm)

Mastra Memory provider implementing the `MastraMemory` interface.

**Key methods:**

```ts
const mem = new SynapseMemory({ path: "./agent.syn" })
mem.store(threadId, role, content)
mem.recall(threadId, query, topK?)
mem.delete(threadId)
mem.export()            // returns CRDT-mergeable snapshot
mem.import(snapshot)
```

**Repo structure:** `adapters/ts/mastra/` — single `index.ts`, exports `SynapseMemory`.

**Install:** `npm install @synapse/mastra`

---

### vercel/ai useMemory provider

Implements the `MemoryProvider` interface from `ai` SDK.

**Key methods:**

```ts
const memory = synapseMemory({ path: "./agent.syn" })
// passes as: experimental_continueConversation({ memory })
memory.get(key)
memory.set(key, value, ttl?)
memory.search(embedding, topK?)
memory.delete(key)
memory.flush()
```

**Repo structure:** `adapters/ts/vercel-ai/` — `index.ts` + `types.ts`.

**Install:** `npm install @synapse/vercel-ai`

---

## 3. Tier-2 — Next 90 Days

**CopilotKit (30k):** `@synapse/copilotkit` — wraps `useCopilotReadable` + `useCopilotAction` to persist and surface agent context from `.syn`. Adapter reads FTS5 hits, injects as readable context.

**browser-use (84k):** `synapse-browser-use` (PyPI) — session memory plugin. Stores page summaries, entity extractions, visited URLs. Hooks into `BrowserSession` lifecycle events.

**e2b (12k):** `synapse-e2b` — sandbox-persistent memory. Mounts `.syn` file into e2b sandbox via their filesystem API; agent reads/writes memory across sandbox restarts.

**screenpipe (18k):** `synapse-screenpipe` — ingests screenpipe JSONL events into `.syn` episodic store. CLI: `screenpipe-events | synapse ingest --source screenpipe`.

---

## 4. Validation / Eval

Target tools: **promptfoo (20k)**, **mlflow (25k)**, **langfuse (23k)**.

Ship `bench/promptfoo.yaml` checked into the repo:

```yaml
# bench/promptfoo.yaml
providers:
  - id: synapse
    config:
      path: bench/fixtures/test.syn
tests:
  - description: Exact recall after 1 insert
    vars: { query: "user birthday" }
    assert:
      - type: contains
        value: "1990"
  - description: Fuzzy recall across 1000 docs (p95 < 10ms)
    vars: { query: "project deadline" }
    assert:
      - type: latency
        threshold: 10
```

MLflow: log retrieval latency, MRR@10, recall@5 as metrics per release. Langfuse: trace MCP `memory/search` calls in production integrations.

---

## 5. Competitors — Positioning

| Competitor | Bench status | Headline |
|---|---|---|
| memvid | done (9074× slower) | single-file wins |
| qdrant | done | no separate process |
| **mem0** | ADD — measure add+search latency vs synapse-mem0 shim | same API, 100× faster |
| llmware | skip | different segment (doc pipelines) |

---

## 6. GTM Multipliers

**wasp-lang template:** `synapse-wasp-starter` — Wasp full-stack app with Synapse memory wired to AI actions. One-click deploy. Listed in Wasp template registry.

**WrenAI / Canner text-to-SQL:** Point WrenAI at `.syn` files — users query their own agent memory in natural language. Demo: `wren connect --file agent.syn` → "What did the agent do last Tuesday?"

---

## 7. Skip-List

ClickHouse, TimescaleDB, VictoriaMetrics, Prometheus exporters, Grafana dashboards, Kubernetes operators, Helm charts — wrong segment. Synapse targets single-agent / small-fleet OSS tooling, not observability infrastructure.

---

## 8. Release Cadence

| Version | Milestone |
|---|---|
| **v0.3.1** | `synapse-mem0` PyPI shim + mem0 bench added |
| **v0.4** | TS SDK: `@synapse/mastra` + `@synapse/vercel-ai` |
| **v0.5** | Eval suite: `bench/promptfoo.yaml` + mlflow metrics |
| **v1.0** | All Tier-1 adapters shipped + Tier-2 at beta |

---

## 9. Community Plays

- **HN:** "Show HN: Synapse — mem0-compatible agent memory, 1 SQLite file, self-host, 9000× faster than memvid"
- **Reddit:** r/LocalLLaMA (self-host angle), r/rust (Rust + SQLite engineering story)
- **Awesome-lists PRs:** `awesome-rust`, `awesome-mcp-servers`, `awesome-ai-agents`
- **Discord:** Post adapter announcements in Mastra `#integrations` + Vercel AI `#community-packages`
- **PyPI / npm READMEs:** Each adapter README links back to main repo + bench results

---

## 10. Metrics to Track

| Metric | Tool | Target (v1.0) |
|---|---|---|
| GitHub stars | gh api | 5k |
| PyPI downloads/week | pypistats | 10k |
| npm downloads/week | npm stats | 5k |
| Integration PRs merged upstream | gh search | 3 (mem0, mastra, vercel/ai) |
| Retrieval latency p95 | bench/promptfoo.yaml CI | < 10ms |
