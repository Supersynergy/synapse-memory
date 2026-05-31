# Architecture Notes 2026-05-16

## Benchmark Verdict

Latest local run after the indexed scoped-recall patch and claude-mem/synapse-mem0 adapters:

- Report: `results/recall_bakeoff_20260516-124537.md`
- Raw: `results/recall_bakeoff_20260516-124537.json`
- Corpus: 8 unique facts, 16 queries, top-k 5

Results:

| Engine | R@5 | p95 |
|---|---:|---:|
| Synapse scoped fusion SDK | 1.000 | 0.99 ms |
| Cognee session recall | 1.000 | 7.32 ms |
| Synapse mem0-compatible socket | 1.000 | 67.98 ms |
| Signet daemon CLI | 1.000 | 486.63 ms |
| Synapse hybrid socket | 0.938 | 71.16 ms |
| SQLite FTS5 control | 0.625 | 0.04 ms |
| Mem0 FAISS mock local | 0.625 | 0.62 ms |
| claude-mem worker FTS | 0.000 | 1.30 ms |
| Letta archive passages | 0.000 | 63.46 ms |
| OMEGA CLI | 0.000 | 551.07 ms |

Interpretation:

- Synapse scoped recall now wins this local recall gate on latency while matching perfect R@5. The decisive fix is indexed scope-first retrieval, not more global vector work.
- Cognee remains strongest on MRR in this tiny session-memory test, but it used session recall, not the full graph path, and is about 7x slower at p95 than Synapse scoped.
- Signet retrieved all expected facts but paid a CLI/daemon cost around 0.49 seconds p95.
- The Synapse mem0-compatible facade now has real scoped SQL-first recall and reaches R@5 1.0, but the compatibility API is still much slower than native AgentDB/scoped recall.
- Synapse default hybrid ran inside the real global brain with about 295k docs. Its miss pattern is mostly ranking/scope noise, not inability to store or search.
- claude-mem is now benchmarked as a real isolated worker via `/api/import` plus `/api/search?format=json`. Its SQLite FTS path is fast but fails this bag-of-words query set because its search path is phrase-heavy.
- Letta archive writes worked, but local `/v1/passages/search` returned no hits in this setup; treat Letta as an agent-memory architecture reference until its archival search path is wired.
- OMEGA is useful for lifecycle ideas, but its CLI path is not a hot recall path in this local setup.

## Best Architecture For Synapse

Keep Synapse as the hot local recall kernel and add an evidence router around it.

```text
User/task query
-> query classifier: code/docs/memory/decision/error/version
-> scope resolver: project/session/package/version
-> Synapse hot recall: FTS5 + vec + recency + exact anchors
-> learned reranker: accepted context + opened files + edits + test outcome
-> optional enrichment:
   - Cognee/Graphiti for temporal graph and contradiction synthesis
   - Signet for portable identity/structured memory patterns
   - Letta for core-vs-archival memory UX
   - Context7/Docfork/GitMCP/DeepWiki for current docs evidence
-> context packer: token budget, dedupe, freshness, citations
```

## Patterns To Steal

1. From Cognee: session-local recall should be first-class. Synapse needs cheap project/session scopes that can be filtered before global ranking.
2. From Signet: memory should carry identity, hints, importance, provenance, and structured expansion paths.
3. From Letta: split pinned core memory from archival recall. Do not let long-tail archival results crowd out working-set facts.
4. From OMEGA: add validity and lifecycle operations: supersedes, contradiction, forget, consolidate.
5. From Context7/GitMCP/DeepWiki: freshness is evidence, not memory. Resolve installed versions and docs before answering API/package questions.
6. From SQLite/FTS5: lexical is still the fastest fallback. Every semantic path should degrade to lexical safely.

## Concrete Synapse Upgrades

Priority 1: Scoped hot recall

- Done for the SDK hot path: `Bank.recall()` now uses indexed scope-first recall, metadata hydration, BatchSearch fallback, and query-term rerank.
- Added a persistent SQLite expression index on `json_extract(meta, '$.scope')`.
- Done for the mem0-compatible shim: search and get_all now scope by `meta.user_id` in SQL before ranking, preventing large global-brain misses.
- Remaining daemon-native upgrade: add a first-class `SearchScoped` request and return `meta` directly in `Hit`.
- Prefer exact run/project/package anchors before broad global RRF.

Priority 2: Batch keepalive

- Add or finish `synx batch hybrid` for hook chains.
- One process/socket per prompt lifecycle, not one fork per query.

Priority 3: Outcome-trained reranker

- Log query, returned ids, accepted ids, opened files, edits, tests, and final task success.
- Train a small LambdaMART/LightGBM or linear ranker on:
  - fts score
  - vec score
  - exact anchor match
  - scope match
  - recency
  - memory type
  - prior acceptance rate
  - contradiction/superseded flags

Priority 4: Freshness router

- For packages/APIs/models: query local version/doc adapters before memory recall.
- Store verified doc snippets in Synapse with source/version/time metadata.
- Penalize stale snippets unless no fresh evidence exists.

Priority 5: Lifecycle graph

- Minimal local graph is enough:
  - `supersedes`
  - `contradicts`
  - `derived_from`
  - `valid_from`
  - `valid_until`
  - `source_uri`
- Graphiti-style temporal reasoning can run as async enrichment, not in the prompt hot path.

## Why This Beats A Single Tool

Single tools optimize one axis. Synapse can win by routing:

- Hot recall: Synapse.
- Session locality: Cognee pattern.
- Agent memory UX: Letta pattern.
- Identity/provenance: Signet pattern.
- Lifecycle/contradiction: OMEGA/Graphiti pattern.
- Fresh docs: Context7/Docfork/GitMCP/DeepWiki pattern.

The moat is not another vector DB. The moat is a local-first recall kernel with scoped routing, freshness evidence, outcome feedback, and token-budgeted context packing.
