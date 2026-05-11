# agent_memory — Mem0/Letta-killer

Demonstrates Synapse as a **typed, scoped agent memory store** — the primitive
that powers Mem0, Letta, and OpenAI's memory layer, but in a single SQLite file
with sub-millisecond recall.

## What it shows

| Feature | How |
|---------|-----|
| Multi-scope memories | JSON meta `user_id` / `session_id` / `scope` stored per doc |
| Hybrid recall | BM25 (FTS5) + vector RRF fusion → relevant turns surface above noise |
| Scope isolation | post-filter by `user_id` — each user sees only their own history |
| Zero infra | single `brain.db` SQLite file, no server, no Docker |

## Run

```bash
cd examples/agent_memory
cargo run -- demo
# → records 5 chat turns for "alice", queries "remind me what I said about Rust"
# → top hits are Rust turns, not the pizza tangent

cargo run -- recall alice "async runtime"
# → searches alice's memories for async-related turns
```

## Output (demo)

```
── Recording 5 turns  user=alice  session=s001 ──
  stored id=1  [user] I've been exploring Rust for systems programming...
  stored id=2  [assistant] Rust's ownership model eliminates whole class...
  stored id=3  [user] Yeah. I also started learning async Rust with Tok...
  stored id=4  [assistant] Tokio is the go-to async runtime. tokio::spaw...
  stored id=5  [user] By the way, what's a good pizza place in Berlin?
Ingest: 3.2ms

── Recall  query="remind me what I said about Rust"  scope=alice ──
Recall: 1.1ms  3 hits

  #1 score=0.9821  [user] I've been exploring Rust for systems programming...
  #2 score=0.8744  [user] Yeah. I also started learning async Rust with Tokio...
  #3 score=0.7102  [assistant] Rust's ownership model eliminates whole classes...

✓ demo passed — top hit is Rust-relevant
```

## Production upgrade path

1. Replace `pseudo_embed` with `fastembed::Embedder` (BGE-small-en, 384-d, ~20ms cold)
2. Use `SearchOptions { filter: MetadataPredicate { key: "user_id", op: Eq, value: … } }` for DB-level pushdown instead of post-filter
3. Add `MemoryType` tags via `store.put_memory()` for typed retrieval (Fact / Decision / Preference / Episodic)
4. Call `store.sota_migrate()` to enable entity graph + PPR-based recall
