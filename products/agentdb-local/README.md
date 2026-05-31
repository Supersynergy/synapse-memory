# Synapse AgentDB Local

Local-first agent memory database for coding agents.

## Product Promise

Run one local daemon and give agents scoped, token-efficient recall in
milliseconds. No cloud memory service, no hosted vector DB, no project context
dumping.

## Included

- `synapsed`: local daemon over Unix socket.
- `synx-fast`: low-latency CLI for `doctor`, scoped search, compact context,
  scoped writes, and keepalive batch.
- Local installer and Docker runtime files.
- Smoke test that starts a real temporary daemon and verifies scoped search,
  context packing, and batch.

## First Run From Monorepo

```bash
./install.sh --local
synapsed --file ~/.synapse/brain.db --sock /tmp/synapse.sock --lazy-embed
SYNAPSE_SOCK=/tmp/synapse.sock synx-fast doctor
synx-fast put --scope my-project --title decision "Use scoped recall for agent context"
synx-fast context --scope my-project "scoped recall agent context" --budget 600
```

## Product Verify

```bash
products/agentdb-local/scripts/verify.sh
```

Expected signal:

- Rust daemon and fast CLI compile.
- `make smoke-fast` passes against a real temporary daemon.

## Canonical Positioning

Sell this as: "SQLite-simple local AgentDB with scoped memory recall for coding
agents." Do not lead with generic vector database. The wedge is faster, cleaner
agent context.
