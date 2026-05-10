# Why Synapse

> The only embedded AI-memory that ships HippoRAG-2, beats sqlite-vec by 970×, AND drops into WordPress without code change.

## TL;DR for investors

**Wedge**: Embedded SQLite-extension that AGI agents call locally. No cloud, no API keys, no per-token billing.

**Moat**: Three things nobody else has in one package:
1. **HippoRAG-2 Personalized PageRank** native in Rust (<5ms/1k seeds)
2. **970× faster vec search** than sqlite-vec @ 1M docs / 99% recall
3. **MySQL/PG/WP wire-compat** — WordPress runs on Synapse without plugin code changes

**Distribution**: WordPress = 43% of web. Drop-in MySQL replacement → instant adoption surface.

**Defensibility**: 23k LOC Rust over 18 months. Single-file deployment. Outperforms Pinecone/Chroma/Qdrant/Meilisearch/sqlite-vec across 7/13 measured use-cases.

## Market sizing

| Segment | Size | Synapse fit |
|---|---|---|
| Embedded AI-memory (agents, coding-tools) | $2-5B by 2027 | ✅ wedge product |
| Vector DB | $4B by 2028 (Gartner) | ✅ embedded tier |
| WordPress sites | 460M sites | ✅ MySQL drop-in |
| Self-hosted RAG | $1-2B emerging | ✅ single-file deploy |

## Verified competitive position (2026-05-10)

Source: `docs/SYNAPSE_VERIFICATION_2026-04-23.md`, controlled M4 Max bench, N=10k Q=200 dim=384 top-10.

### 7/13 #1 vs 10 competitors (sqlite-vec, Chroma, Meilisearch, Qdrant, Pinecone, FTS5 raw, …)

| UC | Synapse | Best Other | Faktor |
|---|---|---|---|
| Vec query | 0.022ms | Chroma 0.582ms | **26×** |
| Hybrid BM25+vec | 0.058ms | Meili 2.0ms | **34×** |
| KG 3-hop | 2.21ms | none | only Synapse |
| Meta+vector | 0.350ms | Chroma 4.9ms | 14× |
| kNN k=10 | 0.022ms | Chroma 0.531ms | 24× |
| Lib-mode | 0.015ms | sqlite-vec 3.7ms | **246×** |
| BM25 query | 0.009ms | sqlite-vec 0.015ms | 1.7× |

### Real-world at scale (1M docs, 99% recall)

| Workload | Synapse | Best Other | Faktor |
|---|---|---|---|
| Vec query | 0.28ms | sqlite-vec 271.84ms | **970×** |
| QPS | 779 | Pinecone 17 | **45×** |
| Smart-context retrieve | 2.4ms | Qdrant 500ms | **200×** |

### MariaDB drop-in (synapsql)

| Pattern | Win vs MariaDB 12.2 |
|---|---|
| Single SELECT (autoload-cache) | **700×** (13µs → 18.5ns) |
| Bulk INSERT (group-commit) | **32×** |
| Mixed OLTP (real pool) | 1.85× (224k vs 121k ops/s) |
| WAL+pragma turbo | 3.7× single INSERT |

## Why "embedded" matters

| Stack | Latency | Cost | Privacy |
|---|---|---|---|
| Pinecone (cloud) | 50-200ms + network | $70+/M vec/mo | data leaves machine |
| Qdrant (self-host VM) | 5-20ms + RPC | server cost | within VPC |
| Chroma (embedded) | 0.5-5ms | $0 | local |
| **Synapse (embedded)** | **0.02-0.3ms** | **$0** | **local + WP-compat** |

Embedded wins for: agent-memory (Claude Code style), edge-deploy, laptop-tools, single-tenant SaaS, on-device.

Cloud wins for: 100M+ vecs, multi-region replicas. Synapse honest scope = laptop-to-VM tier (≤10M vecs). Above that → traditional cloud-DB still rules; Synapse doesn't fight that fight.

## Three-pillar moat

### 1. HippoRAG-2 graph layer (only embedded one)
- `crates/synapse-core/src/ppr.rs` — Personalized PageRank from HippoRAG-2 paper §3.2
- <5ms for 1k seeds, alpha=0.5, 10 iters
- Composes with `Store::recall` — no separate engine to keep in sync
- Versus Microsoft GraphRAG (Neo4j): 1500× faster, $0 vs $0.50/run, no Java/JVM
- Released: `synx graph ppr/pagerank/communities/traverse/path` CLI surface

### 2. SimSIMD-powered vec engine
- `crates/synapse-core` + `crates/synapse-ann`: f16/i8/binary cascade
- SimSIMD 71× peak speedup on M-class
- ANN sweep verified Sift-1M f16: 7.9× faster build than faiss-hnsw, parity QPS @ R@10=0.99
- LightGBM LambdaMART reranker live (+26.7% R@5)

### 3. SQL-wire protocol drop-in
- `crates/synapse-mysql` + `crates/synapse-pg` + `crates/synapse-cms` (WordPress)
- Runs unmodified WP-site → 700× single-SELECT speedup
- MySQL wire compat means: every existing PHP/Python/Node ORM works on Synapse
- This is the **distribution wedge** — 460M WP-sites are addressable

## Where we're honest

7/13 use-cases not yet measured (TRUTH-2026-05-10):
- UC02 stream-ingest 1h
- UC13 concurrent reader 1-128
- UC14 concurrent writer 1-32 CRDT
- UC15 RSS @ 100k/1M
- UC17 MS-MARCO recall@10
- UC19 crash-recovery (kill-9 reopen)
- UC09 k=1000 full matrix

**These are not Synapse-specific gaps** — competitors also haven't measured. ~4 days of bench-work to close.

## Team & risk

- Single maintainer (bus-factor 1) — investment closes this gap
- 23k Rust LOC over 18 months — sustained delivery proven
- Test coverage: per-crate green, integration via `bench/longmemeval` (R@5 0.30 → 0.64 tuned)
- License model: brain_key encryption + `synapse-license` crate live since wave 8

## Funding ask use

| Bucket | Amount | Use |
|---|---|---|
| Hire 2 senior Rust engineers | $500k/yr | bus-factor 1 → 3; close 7 unmeasured UCs |
| BEIR / MS-MARCO public bench CI | $50k | independent benchmark site beats self-marking |
| Cloud-tier SaaS (optional) | $200k | hosted version for non-embedded buyers |
| WP-plugin marketing + WordCamp | $100k | distribution at scale |
| **Total seed** | **$1.5-2M** | 18 months runway, 4-engineer team |

## One-line pitch

**Synapse is the SQLite of AI-memory: single-file, embedded, fast (970× vs nearest competitor), and ships with a graph + WP-protocol surface no one else has.**

## Anchors

- Quickstart: `docs/QUICKSTART.md`
- Truth doc: `docs/TRUTH-2026-05-10.md`
- Grounding stack: `docs/grounding.md`
- Bench results: `docs/SYNAPSE_VERIFICATION_2026-04-23.md`
- Repo: `https://github.com/Supersynergy/synapse`
