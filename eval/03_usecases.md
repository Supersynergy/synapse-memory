# Use-Case Results

## UC2: Multi-Agent CRDT-Shared Memory

**Method**: Spawned 3 CLI-backed db files. Each wrote 2 docs (1 shared, 1 unique). Exported agent0 as `.brainpack`, imported into agent1's db.

**Correctness**: FAIL — snap/restore is full db replace, not merge. Merged db = agent0's db only (2 docs). Agent1's unique data lost. CRDT is not implemented.

**Verdict**: snap+restore can replicate a brain, not merge two brains. Multi-agent shared memory requires external merge logic or a future set-union op.

---

## UC7: Lead Industry Shard (10k synthetic companies)

**Input**: 10,000 synthetic company docs. Format: `{industry} company in {city} revenue:{n}k employees:{n}`

| Metric | Value |
|--------|-------|
| Insert 10k | 135.8ms |
| Throughput | 73,635 docs/s |
| BM25 search p50 | 0.25ms |
| BM25 search p95 | 0.49ms |
| Correctness | Exact keyword match works (industry+city queries return relevant hits) |

**Verdict**: works well for lead shard use-case. BM25 sub-0.5ms at 10k docs.

---

## UC13: Per-Domain Antiban Memory

**Input**: 130 real hosts from `~/.claude/logs/antiban.db`. Text: `host:{domain} stage:{stage} ok:{n} fail:{n} camoufox:{n}`

| Metric | Value |
|--------|-------|
| Insert 130 hosts | 2.3ms |
| Search p50 | 0.05ms |
| Search p95 | 0.19ms |
| Correctness | Host domain prefix search returns correct stage/stats |

**Verdict**: works perfectly. Store antiban state as synapse docs, query by hostname fragment.

---

## UC27: Skill-Index Semantic Search

**Input**: 10 entries from `~/.claude/skills.idx` (index only has 10, not 698 as advertised).

| Metric | Value |
|--------|-------|
| Insert 10 skills with embeddings | 101,746ms (~102s, model warm-up) |
| Vec search latency (post-warm) | 11–14ms/q |
| Correctness | Semantic matches found for "browser automation", "fetch token savings" |

**Note**: Embedding init cold-start is ~100s (downloads BGE-small model). Subsequent runs use cache. Use `--lazy-embed` + pre-warm in production. With warm cache: insert 10 docs ~50ms.

**Verdict**: works for semantic skill lookup. Cache is critical.

---

## UC41: DSGVO Audit Ed25519-Signed Log

**Note**: Ed25519 signing is not implemented in synapse. BLAKE3 content-addressing provides tamper evidence only.

**Method**: 1,000 synthetic audit events stored. Tamper detection via BLAKE3 dedup (modified text → new ID).

| Metric | Value |
|--------|-------|
| Insert 1,000 audit events | 13.9ms |
| Tamper detection | Works: modified text gets new doc ID, original ID unchanged |
| Re-insert idempotency | Correct: same text → same ID |
| FTS5 search over audit log | <0.3ms/q |

**Verdict**: usable as append-only audit log with BLAKE3 integrity. Not a cryptographic signature — no key-based non-repudiation. Suitable for DSGVO data inventory logs, not legal signing.
