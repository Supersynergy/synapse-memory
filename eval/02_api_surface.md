# API Surface

## Core Types (`synapse-core/src/types.rs`)

```rust
pub struct PutRequest {
    pub uri: Option<String>,
    pub title: Option<String>,
    pub text: String,                        // required
    pub meta: Option<serde_json::Value>,
    pub embedding: Option<Vec<f32>>,         // 384-dim BGE-small
}

pub enum SearchMode { Lex, Vec, Hybrid }

pub struct Hit { pub id: i64, pub uri: Option<String>, pub title: Option<String>,
                 pub text: String, pub score: f64 }

pub struct Stats { pub docs: i64, pub vecs: i64 }
```

## Store API (`synapse-core/src/db.rs`)

```rust
Store::open(path) -> Result<Store>
store.put(req: &PutRequest) -> Result<i64>          // BLAKE3 dedup, returns existing id on collision
store.put_batch(reqs: &[PutRequest]) -> Result<Vec<i64>>
store.get(id: i64) -> Result<Doc>
store.search(q, mode, query_emb: Option<&[f32]>, limit) -> Result<Vec<Hit>>
store.stats() -> Result<Stats>
snap::export(db_path, out, level: i32) -> Result<()>  // zstd-packed SQLite backup
snap::import(pack, out) -> Result<()>                  // full db replace (NOT merge)
```

## CLI (`synapse-cli`)

```bash
synapse -f brain.db init
synapse -f brain.db put --text "..." --title "..." [--no-embed]
synapse -f brain.db put --title "..." < stdin
synapse -f brain.db find "query" [--limit 10]   # Lex FTS5
synapse -f brain.db vec "query" [--limit 10]    # kNN
synapse -f brain.db hybrid "query" [--limit 10] # RRF fusion
synapse -f brain.db stats
synapse -f brain.db snap out.brainpack [--level 3]
synapse -f brain.db restore pack.brainpack
```

## Daemon RPC (`synapsed` — msgpack over AF_UNIX)

```
Socket: /tmp/synapse.sock (default)
Protocol: length-prefixed msgpack

Ops:
  {"op": "Ping"}
  {"op": "Put", "args": {title, uri, text, meta, embed: bool}}
  {"op": "PutBatch", "args": [{...},...]}
  {"op": "Search", "args": {mode: "Lex"|"Vec"|"Hybrid", q, limit, embed_query: bool}}
  {"op": "Stats"}
  {"op": "Snap", "args": {out: path, level: 3}}
```

Python client: `bench/client.py` — 40 lines, `pip install msgpack`.

## MCP (`synapse-mcp`)

Stdio JSON-RPC bridge. Tools exposed: `put`, `search`, `stats`.
Config:
```json
{"mcpServers": {"synapse": {"command": "/path/synapse-mcp", "args": ["--sock", "/tmp/synapse.sock"]}}}
```

## Missing / Not Implemented

- **Ed25519 signing**: not in any crate. BLAKE3 dedup provides tamper evidence, not cryptographic signing.
- **CRDT merge**: snap/restore is full db replace, not set-union. No true CRDT.
- **Temporal KG**: no graph layer. Plain SQLite + `ts` timestamp column only.
- **Sharding**: no built-in shard routing. Must implement externally.
