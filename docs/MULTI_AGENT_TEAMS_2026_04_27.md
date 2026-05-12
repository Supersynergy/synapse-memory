# Multi-Agent Teams on Synapse Cognition Bus

**Date:** 2026-04-27
**Author:** Maxim Supersynergy
**Substrate:** `~/.synapse/cognition.db` (SQLite triple-store, bi-temporal) + intent-manager v0.2 + MLX daemon + 4-tier cascade
**Goal:** Make Synapse the SHARED MEMORY + COORDINATION BUS so heterogeneous agent stacks (Claude Code subagents, ZeroClaw, OpenFang, ruflo, Cursor, Devin, AutoGen, CrewAI, LangGraph, OpenAI Swarm, MCP-clients) orchestrate like Claude Code's native Task-tool — parallel spawn, SendMessage, TeamCreate — but cross-tool.

---

## TL;DR
- **Protocol bet:** MCP-as-transport + A2A-AgentCard-as-discovery. ANP/AGNTCY too DID-heavy. OpenAI Agent-Protocol dead. Wrap everything else as MCP servers that read/write cognition.db.
- **Coordination primitive:** SQLite row = task. CAS claim via `UPDATE … WHERE owner IS NULL`. WAL + busy_timeout. No external broker.
- **Pub/sub:** SQLite `update_hook` in intent-manager daemon → in-proc fanout → unix socket subscribers. Polling fallback for non-Rust clients (200ms).
- **Capability registry:** new tables `agent`, `capability`, `agent_capability` — AgentCard-compatible JSON column, FTS5 + vec for semantic routing.
- **14-day ship:** D1–3 schema+claim, D4–6 MCP bridge, D7–9 adapters (AutoGen/CrewAI/LangGraph), D10–12 SendMessage+TeamCreate, D13–14 bench+doc.

---

## 1. Protocol Comparison (8 dims)

| Dim | **MCP** (Anthropic) | **A2A** (Google) | **ANP** (agent-network-protocol) | **OpenAI Agent-Proto** | **AGNTCY** (Cisco/LF) |
|---|---|---|---|---|---|
| Transport | stdio/SSE/HTTP | HTTP/SSE/JSON-RPC | HTTPS + WSS | HTTP REST | gRPC + NATS |
| Discovery | `tools/list` per server | **AgentCard** (`/.well-known/agent.json`) | DID:wba + JSON-LD descriptor | OpenAPI | Directory svc |
| Identity | none (trust transport) | optional OIDC | DID-WBA (W3C DID) | API key | mTLS+SPIFFE |
| State | client-managed | task object (server) | message history | thread/run | session |
| Streaming | yes (SSE) | yes (SSE) | yes (WSS) | yes (SSE) | gRPC stream |
| Multi-agent prim | none (1:1) | task delegation | peer mesh | handoff | swarm/topology |
| Adoption (2026-04) | **huge** (Claude/Cursor/Cody/Zed) | growing (Google ADK, LangGraph adapter, A2A SDK) | tiny (CN-led) | abandoned | early enterprise |
| Shippable today | ✅ stdio in 50 LOC | ✅ FastAPI server + AgentCard JSON | ⚠ DID infra needed | ❌ deprecated | ⚠ NATS dep |

**Verdict:** ship MCP-server-on-cognition.db NOW. Add `/.well-known/agent.json` (A2A AgentCard) on the same HTTP :9477 for cross-tool discovery. Skip ANP/AGNTCY until customer demand. Skip OpenAI-AP entirely.

---

## 2. Capability Registry — DDL (add to cognition.db)

```sql
-- AGENTS: anything that can do work. Claude subagent, ZeroClaw worker, AutoGen role, etc.
CREATE TABLE IF NOT EXISTS agent (
  id            TEXT PRIMARY KEY,              -- 'claude:planner', 'zeroclaw:scraper-7', 'autogen:critic'
  kind          TEXT NOT NULL,                 -- 'claude_subagent'|'zeroclaw'|'autogen'|'crewai'|'langgraph'|'mcp'|'http'
  endpoint      TEXT,                          -- mcp:stdio:cmd, http://..., unix:/tmp/... , tasktool://
  agent_card    TEXT NOT NULL,                 -- JSON: A2A AgentCard (name, description, version, skills[], auth)
  status        TEXT NOT NULL DEFAULT 'idle',  -- idle|busy|offline|dead
  heartbeat_at  INTEGER,                       -- unix ms; >30s old → offline
  cost_hint     REAL DEFAULT 0,                -- $/task estimate, for router
  latency_p50   INTEGER,                       -- ms, learned
  reliability   REAL DEFAULT 1.0,              -- 0..1 EWMA from outcomes
  created_at    INTEGER NOT NULL,
  valid_from    INTEGER NOT NULL,              -- bi-temporal
  valid_to      INTEGER                        -- NULL = current
);
CREATE INDEX idx_agent_status ON agent(status, kind);

-- CAPABILITIES: declarative skills. Synonyms via aho-corasick (already in intent-manager).
CREATE TABLE IF NOT EXISTS capability (
  id           TEXT PRIMARY KEY,               -- 'web.scrape', 'code.refactor.rust', 'pdf.ocr'
  name         TEXT NOT NULL,
  description  TEXT NOT NULL,
  schema_in    TEXT,                           -- JSON-Schema for input
  schema_out   TEXT,                           -- JSON-Schema for output
  tags         TEXT                            -- csv: 'browser,stealth,bulk'
);

CREATE TABLE IF NOT EXISTS agent_capability (
  agent_id      TEXT NOT NULL REFERENCES agent(id),
  capability_id TEXT NOT NULL REFERENCES capability(id),
  confidence    REAL DEFAULT 1.0,              -- self-declared or learned
  cost_per_call REAL,
  PRIMARY KEY (agent_id, capability_id)
);

-- FTS + VEC for semantic routing (reuse Synapse hybrid stack)
CREATE VIRTUAL TABLE IF NOT EXISTS capability_fts USING fts5(id, name, description, tags, content=capability);
-- vec0 table populated by MLX daemon embedding capability.description on insert
```

**AgentCard JSON convention** (stored in `agent.agent_card`, A2A-compatible):
```json
{
  "name": "zeroclaw-scraper", "version": "0.3", "description": "Stealth web scraping",
  "url": "http://localhost:7301/a2a", "skills": [{"id":"web.scrape","tags":["stealth"]}],
  "authentication": {"schemes":["bearer"]}, "defaultInputModes":["text"], "defaultOutputModes":["application/json"]
}
```
Serve at `:9477/.well-known/agent.json` for any registered agent → instant A2A interop.

---

## 3. Task Claim / Lock (idiomatic SQLite)

```sql
CREATE TABLE IF NOT EXISTS task (
  id            TEXT PRIMARY KEY,             -- ulid
  team_id       TEXT,
  parent_id     TEXT REFERENCES task(id),     -- DAG / spawn tree
  capability_id TEXT NOT NULL,
  payload       BLOB NOT NULL,                -- msgpack
  status        TEXT NOT NULL DEFAULT 'pending', -- pending|claimed|running|done|failed|cancelled
  owner         TEXT REFERENCES agent(id),
  claim_token   TEXT,                         -- random; must match on update → fencing
  attempts      INTEGER DEFAULT 0,
  result        BLOB,
  error         TEXT,
  created_at    INTEGER NOT NULL,
  claimed_at    INTEGER,
  finished_at   INTEGER,
  deadline_at   INTEGER                       -- claim auto-expires; reaper resets
);
CREATE INDEX idx_task_pending ON task(status, capability_id) WHERE status='pending';
CREATE INDEX idx_task_claimed ON task(status, deadline_at) WHERE status='claimed';
```

**Atomic CAS claim** (works on WAL with `busy_timeout=5000`):
```sql
-- agent calls this; one winner.
UPDATE task
   SET status='claimed', owner=?agent, claim_token=?token,
       claimed_at=?now, deadline_at=?now+30000, attempts=attempts+1
 WHERE id = (
   SELECT id FROM task
    WHERE status='pending' AND capability_id IN (SELECT capability_id FROM agent_capability WHERE agent_id=?agent)
    ORDER BY created_at LIMIT 1
 )
   AND status='pending'
RETURNING id, payload;
```
- `RETURNING` (SQLite ≥3.35) gives single-roundtrip claim.
- `claim_token` = fencing token; finish requires matching token → prevents zombie writer overwriting after timeout.
- Reaper: every 5s, `UPDATE task SET status='pending', owner=NULL WHERE status='claimed' AND deadline_at<now`.

**Why not row-locks / advisory locks:** SQLite has no row locks; whole-DB write lock is fine because WAL serializes. CAS pattern is canonical (litequeue, neoq, river-on-sqlite, mq-lite all use it).

---

## 4. Pub/Sub — `update_hook` Fanout

**Choice:** SQLite `sqlite3_update_hook` registered inside intent-manager daemon, callback pushes `(table, rowid, op)` onto a tokio broadcast channel; subscribers attach via unix socket `/tmp/synapse.bus.sock` (msgpack frames). Polling fallback on `task` table for non-attached clients (200ms `SELECT … WHERE updated_at > ?last`).

**Why update_hook over alternatives:**
| Option | Verdict |
|---|---|
| `update_hook` in-proc fanout | ✅ zero-dep, sub-ms, already have rusqlite |
| Postgres `LISTEN/NOTIFY` equivalent | ❌ doesn't exist in SQLite |
| litequeue / pgmq-on-sqlite | ⚠ adds tables for what's already a row update |
| NATS / Redis Streams | ❌ extra process, breaks single-binary story |
| File-watch (inotify on db-wal) | ❌ noisy, no row granularity |
| Cron polling only | ⚠ fine fallback, 200ms latency floor |

**Caveat:** `update_hook` only fires in the connection that did the write. Therefore **all writes route through intent-manager daemon** (HTTP :9477 / unix socket). External MCP clients write via daemon API, never open cognition.db directly. This also gives us auth, audit, and rate-limit for free.

**Event shape** (msgpack):
```
{ev:"task.claimed", task_id:"01H...", agent_id:"autogen:critic", ts:..., team_id:"..."}
{ev:"task.done",    task_id:"01H...", result_ref:"blob:...", ts:...}
{ev:"agent.heartbeat", agent_id:"...", status:"idle", ts:...}
```

---

## 5. Cross-Tool Message Format

**Pick: msgpack on the wire, JSON-Schema for contract, A2A `Message` shape for envelope.**

```jsonc
// envelope (msgpack-encoded; JSON shown for reading)
{
  "v": 1,
  "id": "01HZ...",                 // ulid
  "kind": "task" | "result" | "event" | "ask" | "tell",
  "from": "agent:claude:planner",
  "to":   "agent:zeroclaw:scraper" | "team:42" | "cap:web.scrape",
  "in_reply_to": null,
  "team_id": "team:42",
  "capability": "web.scrape",
  "payload": {...},                // schema = capability.schema_in / out
  "deadline_ms": 30000,
  "trace_id": "..."                // OTel propagation
}
```

**Why:** msgpack already in intent-manager (`rmp-serde`), 30–60% smaller than JSON, zero-copy decodable. Envelope mirrors A2A `Message` so an A2A→Synapse adapter is a field-rename. ChatML/OpenAI are LLM-conversation shapes, not coordination. LangChain Agent-Protocol is dead. JSON-Schema gates `payload` per `capability_id`.

---

## 6. Top-10 Stealable Patterns

| # | Pattern | Repo | Pointer | Steal as |
|---|---|---|---|---|
| 1 | GroupChatManager turn-taking | `microsoft/autogen` | `autogen-core/src/autogen_core/_routed_agent.py` (RoutedAgent + message handler decorator) | `team_chat` table + `next_speaker(team_id)` SQL function |
| 2 | StateGraph + Checkpointer | `langchain-ai/langgraph` | `libs/checkpoint-sqlite/langgraph/checkpoint/sqlite/aio.py` | `task.parent_id` DAG + checkpoint blob per task |
| 3 | Process.hierarchical manager-agent | `crewAIInc/crewAI` | `src/crewai/process.py` + `src/crewai/agents/agent_builder/utilities/` | `team.manager_agent_id` + delegation policy |
| 4 | Swarm handoff function | `openai/swarm` | `swarm/core.py::handle_function_result` (handoff returns Agent) | `result.handoff_to` field → daemon enqueues new task |
| 5 | MetaGPT message-bus | `geekan/MetaGPT` | `metagpt/environment/base_env.py` (`publish_message` / `subscribers`) | our `update_hook` fanout = same shape |
| 6 | A2A AgentCard discovery | `google/A2A` | `specification/json/a2a.json` (AgentCard schema) | copy schema verbatim into `agent.agent_card` |
| 7 | MCP capability listing | `modelcontextprotocol/specification` | `schema/2025-06-18/schema.ts` (`tools/list`) | wrap `agent_capability` as MCP `tools/list` response |
| 8 | LiteQueue CAS claim | `litements/litequeue` | `litequeue.py::pop()` (UPDATE … WHERE id IN SELECT … LIMIT 1) | already used in §3 |
| 9 | River fencing token | `riverqueue/river` | `internal/riverinternaltest/...` (job leadership token) | `claim_token` column in §3 |
| 10 | LangGraph Send / parallel fan-out | `langchain-ai/langgraph` | `libs/langgraph/langgraph/types.py::Send` | `TeamCreate` API: insert N child tasks in one tx |

(Pin exact commits in adapter code — these are HEAD pointers as of 2026-04. Lock when implementing.)

---

## 7. 14-Day Sprint

**D1 — Schema + migration**
- Add tables `agent`, `capability`, `agent_capability`, `task`, `team`, `team_member`, `event_log` to `cognition.db`.
- Migration in `intent-manager/migrations/0003_coordination.sql`.

**D2 — Claim API + reaper**
- `POST /v1/task/claim` (CAS UPDATE … RETURNING).
- `POST /v1/task/finish` (token-fenced).
- Reaper task in daemon, 5s tick.

**D3 — update_hook → unix socket bus**
- `rusqlite::Connection::update_hook(...)`.
- `tokio::sync::broadcast` + unix socket `/tmp/synapse.bus.sock`, msgpack frames.
- `intent bus tail` CLI subscribes for debugging.

**D4 — MCP server adapter**
- `intent-manager-mcp` bin: stdio MCP server exposing `task.claim`, `task.finish`, `agent.register`, `cap.search`, `bus.subscribe` as tools.
- Drop-in for Claude Code, Cursor, Cody, Zed.

**D5 — A2A AgentCard endpoint**
- `GET /.well-known/agent.json` on :9477 returns merged capability list as A2A AgentCard.
- `POST /a2a/tasks/send` translates A2A → internal task row.

**D6 — Capability semantic router**
- Reuse 4-tier cascade: Tier1 aho-corasick on `capability.tags`, Tier2 FTS5 on description, Tier3 vec0 cosine, Tier4 MLX rerank.
- `cap.match(query, k=5)` returns ranked `agent_id`s.

**D7 — AutoGen adapter**
- `synapse_autogen` Python pkg: `SynapseGroupChat(GroupChatManager)` writes turns as tasks, `next_speaker` reads from cap-router.
- Shim: each AutoGen agent registers as `agent.kind='autogen'`.

**D8 — CrewAI adapter**
- `SynapseProcess(Process)`: `Process.hierarchical` → manager publishes subtasks via daemon; workers `task.claim`.
- Replaces in-proc Python queue with cognition.db.

**D9 — LangGraph adapter**
- `SynapseCheckpointSaver(BaseCheckpointSaver)` writes checkpoints to `task.result` blob.
- `SynapseSend` fan-out helper inserts N child tasks atomically.

**D10 — SendMessage primitive**
- `POST /v1/msg/send {from,to,kind,payload}` → row in `message` table → bus event.
- CLI: `intent send agent:zeroclaw 'scrape https://x'`.

**D11 — TeamCreate primitive**
- `POST /v1/team {name, members:[agent_ids|cap_queries], policy}`.
- Policies: `swarm` (any-claims), `hierarchical` (manager-routes), `parallel` (broadcast).
- Materializes `team`, `team_member`, optional initial fan-out tasks.

**D12 — ZeroClaw + OpenFang + ruflo adapters**
- Tiny Python/Rust shims (~100 LOC each): poll `/v1/task/claim?cap=web.scrape` loop.
- ruflo: register as `kind='ruflo'`, expose own caps.

**D13 — Bench**
- `hyperfine` on: claim-throughput (target >5k claims/s on M4), bus-fanout latency p99 <2ms, AgentCard discovery <10ms, end-to-end SendMessage <5ms.
- Compare vs in-proc AutoGen and vs Redis-backed CrewAI.

**D14 — Demo + docs**
- Demo: 1 Claude planner + 3 ZeroClaw scrapers + 1 LangGraph reducer, all coordinating via cognition.db, parallel fan-out 50 URLs, bus visible in `intent bus tail`.
- Doc: `docs/COORDINATION_BUS.md`, this file as design ref.

---

## Risks & Mitigations
- **SQLite write contention** at >5k tx/s → mitigation: WAL, busy_timeout=5000, batch finishes, single-writer (daemon).
- **Bus subscriber slow consumer** → bounded broadcast channel, drop-oldest, log gap; subscribers can backfill from `event_log` table.
- **Claim deadlock if agent dies mid-task** → fencing token + reaper handles.
- **MCP/A2A spec drift** → pin spec versions in `agent_card.protocol_version`, dual-serve old/new.
- **Cross-tool auth** → per-agent bearer in `agent_card.authentication`; daemon validates before claim/finish.

## Done-When
- ZeroClaw worker, an AutoGen critic, a LangGraph reducer, and a Claude Code subagent all complete a 50-URL fan-out coordinated only via `cognition.db` + `:9477`, with no other broker, in <30s end-to-end.
