# Established-memory gap

Checked against primary project documentation on 2026-07-13.

## Where established tools are stronger

| Product | Strong surface | Install/runtime cost relevant to Synapse |
|---|---|---|
| Mem0 OSS | Adaptive memory, Python/Node SDKs, REST, dashboard, auth, audit log, eval kit | Library setup is light, but current defaults use an OpenAI LLM + embedder; reference server uses Docker, dashboard, API keys, and Postgres/pgvector |
| Zep Graphiti | Temporal facts, validity windows, entity graph, provenance, ontology, mature hybrid graph retrieval | Embedded FalkorDB Lite now removes the mandatory external server on Python 3.12+, but ingestion still needs an LLM/embedder; reference MCP path uses Docker and telemetry defaults on |
| Letta | Full stateful-agent platform, memory blocks, compaction, agent runtime, SDK/API/eval ecosystem | Stronger as an agent runtime, not a drop-in memory CLI. Current docs prefer Agent local/app-server modes; legacy Docker server is unsupported and still needs model + embedding configuration |
| OpenMemory | Standalone local Python/Node SDKs, SQLite, multi-sector memory, decay/reflection, connectors, server/MCP | Much broader cognitive/SDK surface; repository still warns that it is being rewritten and may break |

Primary sources:

- https://docs.mem0.ai/open-source/overview
- https://docs.mem0.ai/open-source/features/overview
- https://github.com/getzep/graphiti
- https://github.com/getzep/graphiti/tree/main/mcp_server
- https://docs.letta.com/guides/docker
- https://github.com/letta-ai/letta
- https://github.com/CaviraOSS/OpenMemory

## Synapse wedge

| Dimension | Defensible advantage after these release gates |
|---|---|
| Install | One checksum-verified 3.19 MiB native Rust binary; no Docker, Python, Node, database server, cloud account, model, or API key |
| Privacy | Local file by default; no telemetry and no provider call on the portable path |
| Coding fidelity | Verbatim paths, error strings, numbers, source ids, and bounded context instead of LLM-written summaries |
| Continuity | Repo prime + typed decisions + feedback + optional crash-safe Codex checkpoint |
| Portability | Native artifacts for macOS/Linux/Windows x64 and ARM64, each verified on native CI |
| Operations | Backup/restore/merge/integrity/signatures in the same CLI and file format |

This proves a smaller, deterministic **coding-agent continuity path**. It does not
prove higher conversational recall. Synapse wins the current wedge on install,
offline operation, exact evidence, measured footprint, and disconnect recovery;
the established tools remain ahead in automatic extraction, temporal semantics,
connectors, SDK breadth, hosted operations, or full agent runtimes.

## Claims forbidden before proof

- “Best AI memory” or “better recall than Mem0/Graphiti/Letta.”
- “Semantic search included everywhere” for the portable build.
- “Temporal Graphiti replacement” while Synapse graph temporal invalidation is not
  tested on the same workload.
- “Works everywhere” until all six native artifact jobs and clean installs pass.
- Benchmark comparisons that mix in-process Synapse with remote competitor services.

## Proof needed to win the category

1. Run one public, reproducible LoCoMo and LongMemEval harness against identical
   models, prompts, corpus, token budget, and hardware.
2. Publish task quality, p50/p95 latency, RAM, disk, install time, first-use time,
   token cost, and failure count—not only vector microbenchmarks.
3. Add automatic typed-memory extraction as an optional ingest stage while keeping
   deterministic raw evidence and user-controlled promotion.
4. Add temporal supersession/invalidation tests for “what is true now” and “what
   was true then.”
5. Provide stable Rust library plus Python and TypeScript bindings only after the
   CLI contract freezes.
