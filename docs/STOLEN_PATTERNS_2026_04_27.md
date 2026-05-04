# Stolen Patterns — Synapse Cognition Layer

Source mining for `intent-manager` (Rust) + `~/.synapse/cognition.db`.
Date: 2026-04-27. Targets: LangGraph, LiteQueue, AutoGen, MCP-Python.

---

## 1. LangGraph SqliteSaver — checkpoint serialization

**Source**: `langchain-ai/langgraph` @ main
Permalink: https://github.com/langchain-ai/langgraph/blob/main/libs/checkpoint-sqlite/langgraph/checkpoint/sqlite/__init__.py

**Key insight (≤50w)**: Composite primary key `(thread_id, checkpoint_ns, checkpoint_id)` lets one DB host many isolated agent threads. WAL + a single `threading.Lock` around `cursor()` makes `check_same_thread=False` safe. Writes split into a sibling `writes` table keyed by `(checkpoint_id, task_id, idx)` so partial step results survive crashes.

**Distilled Rust**:

```rust
// schema (run once)
const SCHEMA: &str = "
PRAGMA journal_mode=WAL;
CREATE TABLE IF NOT EXISTS checkpoints (
    thread_id     TEXT NOT NULL,
    ns            TEXT NOT NULL DEFAULT '',
    checkpoint_id TEXT NOT NULL,
    parent_id     TEXT,
    kind          TEXT,
    blob          BLOB,
    meta          BLOB,
    PRIMARY KEY (thread_id, ns, checkpoint_id)
);
CREATE TABLE IF NOT EXISTS writes (
    thread_id TEXT, ns TEXT, checkpoint_id TEXT,
    task_id TEXT, idx INTEGER,
    channel TEXT, kind TEXT, value BLOB,
    PRIMARY KEY (thread_id, ns, checkpoint_id, task_id, idx)
);";

pub struct Saver { conn: Mutex<Connection> }   // single Mutex == LangGraph's Lock

impl Saver {
    pub fn put(&self, t: &Thread, cp: &Checkpoint) -> Result<()> {
        let c = self.conn.lock().unwrap();
        c.execute(
            "INSERT OR REPLACE INTO checkpoints
             (thread_id, ns, checkpoint_id, parent_id, kind, blob, meta)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![t.id, t.ns, cp.id, cp.parent, cp.kind, cp.blob, cp.meta],
        )?;
        Ok(())
    }
    pub fn latest(&self, t: &Thread) -> Result<Option<Checkpoint>> {
        let c = self.conn.lock().unwrap();
        c.query_row(
            "SELECT checkpoint_id, parent_id, kind, blob, meta FROM checkpoints
             WHERE thread_id=?1 AND ns=?2 ORDER BY checkpoint_id DESC LIMIT 1",
            params![t.id, t.ns], |r| Checkpoint::from_row(r)
        ).optional()
    }
}
```

**What we improve (≤30w)**: `rusqlite::Mutex<Connection>` is cheaper than a Python re-entrant lock; `bincode`/`postcard` instead of `JsonPlusSerializer` cuts blob ~3×; we drop `checkpoint_ns` (single namespace per DB file).

---

## 2. LiteQueue — atomic claim with `UPDATE…RETURNING`

**Source**: `litements/litequeue` @ main, `litequeue.py`
Permalink: https://github.com/litements/litequeue/blob/main/litequeue.py

**Key insight (≤50w)**: No fencing tokens — atomicity comes from a single SQL statement: `UPDATE … SET status=LOCKED WHERE rowid=(SELECT rowid … WHERE status=READY ORDER BY message_id LIMIT 1) RETURNING *`, run inside a `BEGIN IMMEDIATE`. The inner SELECT + outer UPDATE happen in one transaction; SQLite serializes writers, so two workers can never claim the same row. UUIDv7 message_id gives FIFO order without a separate sequence column.

**Distilled Rust**:

```rust
// status: 0=READY 1=LOCKED 2=DONE 3=FAILED
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS q (
    message_id TEXT PRIMARY KEY,    -- UUIDv7 → time-sortable
    payload    BLOB NOT NULL,
    status     INTEGER NOT NULL DEFAULT 0,
    lock_time  INTEGER,
    in_time    INTEGER NOT NULL,
    done_time  INTEGER
);
CREATE INDEX IF NOT EXISTS q_status ON q(status);";

pub fn pop(conn: &Connection) -> Result<Option<Msg>> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let row = tx.query_row(
        "UPDATE q SET status = 1, lock_time = ?1
         WHERE message_id = (
             SELECT message_id FROM q
             WHERE status = 0
             ORDER BY message_id   -- UUIDv7 == arrival order
             LIMIT 1
         )
         RETURNING message_id, payload",
        params![now_ns()],
        |r| Ok(Msg { id: r.get(0)?, payload: r.get(1)? }),
    ).optional()?;
    tx.commit()?;
    Ok(row)
}

pub fn ack(c: &Connection, id: &str) -> Result<()> {
    c.execute("UPDATE q SET status=2, done_time=?1 WHERE message_id=?2",
              params![now_ns(), id])?; Ok(())
}
```

**What we improve (≤30w)**: Add a visibility-timeout sweeper (`UPDATE … SET status=0 WHERE status=1 AND lock_time<now-30s`) so crashed workers don't leak tasks — LiteQueue requires manual recovery.

---

## 3. AutoGen SelectorGroupChat — speaker selection

**Source**: `microsoft/autogen` @ main, `_selector_group_chat.py`
Permalink: https://github.com/microsoft/autogen/blob/main/python/packages/autogen-agentchat/src/autogen_agentchat/teams/_group_chat/_selector_group_chat.py

**Key insight (≤50w)**: Three-tier fallback: (1) user-supplied `selector_func(thread) -> Optional[str]` short-circuits everything (rule-based); (2) else format `selector_prompt` with `{roles}{participants}{history}` and ask the LLM; (3) parse response with `_mentioned_agents` — a regex over agent names with underscore/space/escape variants. Retry up to `max_selector_attempts` if zero or multiple matches. `allow_repeated_speaker=False` filters previous speaker before LLM call.

**Distilled Rust pseudocode**:

```rust
pub trait SelectorFn: Fn(&[Msg]) -> Option<String> + Send + Sync {}

pub async fn select_speaker(
    thread: &[Msg],
    agents: &[Agent],
    prev: Option<&str>,
    rule: Option<&dyn SelectorFn>,
    llm: &dyn LlmClient,
    prompt_tmpl: &str,
    max_attempts: u8,
) -> Result<String> {
    // 1. rule-based override wins
    if let Some(f) = rule { if let Some(name) = f(thread) { return Ok(name); } }

    // 2. candidate set (no repeats)
    let cands: Vec<&Agent> = agents.iter()
        .filter(|a| Some(a.name.as_str()) != prev).collect();
    if cands.len() == 1 { return Ok(cands[0].name.clone()); }

    let prompt = render(prompt_tmpl, &cands, thread);
    let names: Vec<&str> = cands.iter().map(|a| a.name.as_str()).collect();

    // 3. LLM with retry on ambiguous parse
    for _ in 0..max_attempts {
        let reply = llm.complete(&prompt).await?;
        let hits = mentioned(&reply, &names);   // regex word-boundary count
        if hits.len() == 1 { return Ok(hits.into_iter().next().unwrap().0); }
    }
    Ok(cands[0].name.clone())   // deterministic fallback
}

fn mentioned(text: &str, names: &[&str]) -> HashMap<String, usize> {
    // \b(name|name_with_spaces)\b case-sensitive, count > 0 only
}
```

**What we improve (≤30w)**: Replace regex parse with structured-output JSON (`{"next": "agent_name"}`) — modern models reliably emit it, eliminating retry loop and underscore/space variants entirely.

---

## 4. MCP Python lowlevel Server — JSON-RPC dispatch

**Source**: `modelcontextprotocol/python-sdk` @ main, `src/mcp/server/lowlevel/server.py`
Permalink: https://github.com/modelcontextprotocol/python-sdk/blob/main/src/mcp/server/lowlevel/server.py

**Key insight (≤50w)**: Dispatch is just a `Dict[str, Handler]` populated from `on_*` kwargs at construction. Methods: `ping`, `prompts/list`, `prompts/get`, `resources/list`, `resources/read`, `tools/list`, `tools/call`, `logging/setLevel`, `completion/complete`. `initialize` is handled by the underlying `ServerSession` (not in this dict). Unknown method → `METHOD_NOT_FOUND` error. Notifications go to a parallel `_notification_handlers` dict.

**Distilled Rust**:

```rust
type Handler = Box<dyn Fn(Value) -> BoxFuture<Result<Value>> + Send + Sync>;

pub struct McpServer {
    name: String,
    handlers: HashMap<&'static str, Handler>,
}

impl McpServer {
    pub fn new(name: &str) -> Self {
        Self { name: name.into(), handlers: HashMap::new() }
    }
    pub fn on<F, Fut>(mut self, method: &'static str, f: F) -> Self
    where F: Fn(Value) -> Fut + Send + Sync + 'static,
          Fut: Future<Output = Result<Value>> + Send + 'static {
        self.handlers.insert(method, Box::new(move |p| Box::pin(f(p))));
        self
    }

    /// stdio loop — reads JSON-RPC lines, dispatches, writes responses
    pub async fn run_stdio(self) -> Result<()> {
        let stdin = BufReader::new(tokio::io::stdin()).lines();
        let mut out = tokio::io::stdout();
        tokio::pin!(stdin);
        while let Some(line) = stdin.next_line().await? {
            let req: JsonRpcReq = serde_json::from_str(&line)?;
            let resp = match req.method.as_str() {
                "initialize" => Ok(json!({
                    "protocolVersion": "2025-06-18",
                    "serverInfo": {"name": self.name, "version": "0.1"},
                    "capabilities": {"tools": {}}
                })),
                m => match self.handlers.get(m) {
                    Some(h) => h(req.params.unwrap_or(Value::Null)).await,
                    None => Err(rpc_err(-32601, "Method not found")),
                },
            };
            let line = serde_json::to_string(&JsonRpcResp::new(req.id, resp))?;
            out.write_all(line.as_bytes()).await?;
            out.write_all(b"\n").await?;
        }
        Ok(())
    }
}
```

**What we improve (≤30w)**: Keep `initialize` inline (one method, no separate session object), drop `prompts/`, `resources/subscribe`, `completion/complete` until needed — 4 methods cover 95% of agents (`initialize`, `tools/list`, `tools/call`, `ping`).

---

## Synthesis

The four patterns map cleanly onto Synapse:
- LangGraph schema → `intent_checkpoints` table in `cognition.db`
- LiteQueue claim → `intent_queue` worker dispatch
- AutoGen selector → next-intent routing inside `intent-manager`
- MCP dispatch → cognition-layer's outward-facing tool surface
