# synapse-mcp — Coding-Agent Working Memory

MCP server (stdio JSON-RPC 2.0) that exposes Synapse as a semantic memory backend
for Cursor, Claude Code, Cline, and any MCP-compatible coding agent.

## Tools

| Tool | Description |
|------|-------------|
| `memory_save(text, tags?, title?)` | Persist a code-context snippet |
| `memory_search(query, k=10, mode?)` | Hybrid search (FTS5 + vector) |
| `memory_recent(n=20)` | Last n memories by timestamp |
| `memory_delete(id)` | Tombstone a memory by id |

Low-level tools (`put`, `search`, `timeline`, `merge`, `verify`) also available.

## Latency (M4 Max, 88k docs in brain.db)

| Metric | Value |
|--------|-------|
| p50    | 8 ms  |
| p95    | 70 ms |
| p99    | 89 ms |
| calls/min at p95 | 862 |

100 sequential `memory_search` calls via subprocess (worst-case: new process per call).
With a persistent session the per-call IPC overhead drops to ~1ms (Unix socket round-trip).

## vs Alternatives

| | synapse-mcp | mem0 MCP | basic-memory |
|---|---|---|---|
| p50 search | 8ms | ~200ms (HTTP+OpenAI embed) | ~50ms (SQLite only) |
| Offline | yes | no (OpenAI API) | yes |
| Hybrid FTS+vec | yes | vec only | FTS only |
| CRDT sync | yes | no | no |

## Build

```bash
cargo build --release -p synapse-mcp
```

## Run (requires synapsed)

```bash
# Start synapsed daemon
synapsed --sock /tmp/synapse.sock --file ~/.synapse/brain.db

# Test
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' \
  | ./target/release/synapse-mcp -s /tmp/synapse.sock
```

## Claude Code config (`~/.claude.json`)

```json
{
  "mcpServers": {
    "synapse": {
      "command": "/Users/master/projects/synapse/target/release/synapse-mcp",
      "args": ["-s", "/tmp/synapse.sock"]
    }
  }
}
```

## Cursor config (`~/.cursor/mcp.json`)

```json
{
  "mcpServers": {
    "synapse": {
      "command": "/Users/master/projects/synapse/target/release/synapse-mcp",
      "args": ["-s", "/tmp/synapse.sock"]
    }
  }
}
```

## Benchmark

```bash
./benchmark.sh [/path/to/synapse.sock]
# Ingests 1000 code-context docs, runs 100 memory_search calls, prints p50/p95/p99
```
