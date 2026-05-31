# 04 — MCP Server (Claude Code / Cursor)

Wire Synapse as an MCP tool server so Claude Code and Cursor can search and store memories.

## What you get

Tools exposed to Claude/Cursor:
- `memory_save` — store a memory with tags
- `memory_search` — semantic + keyword recall
- `memory_recent` — latest N memories
- `put` — append a doc
- `search` — lex/vec/hybrid search
- `timeline` — docs ordered by time
- `synapse_merge` — merge a peer brainpack
- `synapse_verify` — verify Ed25519 signature

## Start server

```bash
# Build
cargo build --release -p synapse-mcp

# Run (Unix socket — fastest)
synapse-mcp --sock /tmp/synapse.sock --db ~/.claude/brain.db

# Or TCP (for remote agents)
synapse-mcp --port 9477 --db ~/.claude/brain.db
```

## Wire into Claude Code

Add to `~/.claude/mcp.json`:

```json
{
  "mcpServers": {
    "synapse": {
      "command": "synapse-mcp",
      "args": ["--sock", "/tmp/synapse.sock", "--db", "/Users/you/.claude/brain.db"]
    }
  }
}
```

## Wire into Cursor

Add to `.cursor/mcp.json` in your project root:

```json
{
  "mcpServers": {
    "synapse": {
      "command": "synapse-mcp",
      "args": ["--port", "9477", "--db", "./cursor-memory.db"]
    }
  }
}
```

## Wire into any MCP-compatible agent

```python
# stdio transport
import subprocess, json

proc = subprocess.Popen(
    ["synapse-mcp", "--stdio", "--db", "./agent.db"],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE
)

def call(method, params):
    req = json.dumps({"jsonrpc":"2.0","id":1,"method":method,"params":params}) + "\n"
    proc.stdin.write(req.encode()); proc.stdin.flush()
    return json.loads(proc.stdout.readline())

call("tools/call", {"name": "memory_save", "arguments": {"text": "user prefers dark mode", "tags": ["preference"]}})
results = call("tools/call", {"name": "memory_search", "arguments": {"query": "dark mode", "k": 5}})
print(results)
```

## Run validation script

```bash
python validate.py
```
