# 03 — Function Catalog

Complete catalog of every Synapse function/feature, organized by **product** then **crate**. This is the "all functions documented" deliverable for the synapse-db / synapse-memory / synapse-market split.

Surface tables (CLI, MCP, Python, JS, daemon protocol, SQL-wire) are enumerated exhaustively from source. Library crates list their key public API; `pub_fns` counts are from the workspace inventory (FACTS) and are noted per crate.

**Product map**
- **synapse-memory** — agent-memory product (depends on synapse-db foundation). User surfaces: `synx` CLI, MCP server, Python `synapse`, JS `Synapse`.
- **synapse-db** — engine product = shared foundation + DB surfaces. User surfaces: SQL wire (MySQL :3306 / Postgres :5432 / HTTP :9477), `synapsed` daemon socket, OLAP/TSDB ops.
- **Shared foundation** — `kernel, core, engine, ann, fts, graph, quant, spann, obs`. Lives in synapse-db; synapse-memory consumes via path/submodule.
- **synapse-market** — vertical spinoff (own repo). `smx_*` MCP tools + market lib. The `mcp -> market` dependency is CUT at split time.

---

## 1. synapse-memory — functions

### 1.1 `synx` CLI (crate `synapse-cli`, bin `synapse`/`synx`)

Global flag: `-f, --file <PATH>` (default `.synapse/brain.db`). Source: `crates/synapse-cli/src/main.rs`.

| Command | Args / flags | What it does |
|---|---|---|
| `init` | — | Create/open a new single-file memory store |
| `put` | `--title --uri --text --source --updated --kind --status --meta --no-embed --sign <sk>` (or stdin) | Append a doc; optional embedding + Ed25519 signing; merges metadata into `meta` |
| `remember` | `text --kind --title --uri --freshness --confidence --no-embed` | Store typed memory (decision/fact/preference/bugfix/benchmark/command/session/adr/research/note) with auto-title + provenance meta |
| `find` | `query --limit` | Lexical FTS5 (BM25) search |
| `vec` | `query --limit` | Vector kNN search (server/local embed of query) |
| `hybrid` | `query --limit --guarantee` | RRF fusion lexical+vector; `--guarantee` adds exact brute-force cosine rerank (R@N=1.0) |
| `fallback` | `query --limit` | Search ladder: hybrid → lexical → recent timeline |
| `context` | `query --mode --limit --budget --json` | Compile a bounded, cited context pack (modes: coding/research/decision/debug/daily/auto); kind-prior reranked, logs context query for learning |
| `prime` | `[path] --mode --limit --json` | Repo startup brief: git state, source docs, suggested commands, relevant memories, freshness hints |
| `fresh-context` | `--prompt --cwd --project --mode --json --no-registry --max-registry` | Version-pinned package/API context from local manifests + lockfiles + registry latest lookups |
| `ground` | `query --k --depth --alpha --iters` | One-shot grounding pipeline: hybrid seeds → PPR → graph traverse → JSON bundle |
| `stats` | — | Store statistics (JSON) |
| `doctor` | `--fix --json` | Health check + self-heal hints; `--fix` runs FTS5 optimize |
| `feedback` | `query_id accepted_doc_id --shard_id` | Record positive recall feedback for learned reranking |
| `keygen` | `--sk --vk` | Generate Ed25519 keypair |
| `sign` | `id --sk` | Sign a stored doc by id (blake3(text)) |
| `verify` | `id --vk` | Verify Ed25519 signature of a doc |
| `snap` / `snap-signed` | `out --level [--sk]` | Export `.brainpack` (optionally signed) |
| `restore` | `pack` | Import a `.brainpack` into the file |
| `merge` | `file_a file_b -o out --level` | Merge two brainpacks by URI-match, CRDT-merge `meta_crdt` per doc |
| `merge-snap` | `peer -o out --level` | CRDT merge a peer snapshot into current brain (offline-safe) |
| `backup` | `out --db --level --encrypt --passphrase` | Portable `.synx` export, optional age-passphrase encryption |
| `db-restore` | `pack --db --passphrase` | Import `.synx` into db, idempotent (blake3 dedup) |
| `db-verify` | `--db` | Integrity: verify blake3 of every doc |
| `db-repair` | `--db` | Rebuild FTS5 index from docs |
| `import` | `src --format` | Import docs (csv/tsv/jsonl/json/sqlite/synx/brainpack, parquet feature) |
| `export` | `dst --format` | Export docs to file (synx/brainpack/csv/tsv/jsonl/sqlite/parquet) |
| `convert` | `src dst --from --to` | Import then export (format conversion) |
| `federate add` | `addr --sk` | Add a P2P peer (tcp:host:port or unix:/path) |
| `federate sync` | `--sk peers...` | CRDT sync all known peers |
| `federate peers` | `--sk peers...` | List configured peers |
| `shard split` | `brain -o out_dir --shards` | Split brain.db into N shards (k-means on embeddings) |
| `shard query` | `manifest query --limit` | Query shard manifest: bloom prefilter → centroid-nearest → fan-out → RRF |
| `graph relate` | `from to rel --weight` | Insert/replace a weighted edge |
| `graph pagerank` | `--n --damping --iters` | Top-N nodes by PageRank |
| `graph ppr` | `seeds_json --alpha --iters --limit` | Personalized PageRank from seed map |
| `graph communities` | `--max-iters --top-n` | Label-propagation community detection |
| `graph neighbors` | `node_id --top-k --rel` | Direct neighbors of a node |
| `graph traverse` | `start_id --depth --top-k-per-hop --decay` | Decayed outward traversal |
| `graph path` | `from to --max-depth` | Dijkstra shortest path |
| `graph count` | — | Edge count |
| `learn status` | — | Bandit shard + feedback entry counts |
| `learn consolidate` | — | Near-dup consolidation/merge |
| `learn drift-check` | — | Embedding drift check (needs embed feature) |
| `learn calibrate` | — | Update calibration from feedback log |

### 1.2 MCP tools (crate `synapse-mcp`, stdio JSON-RPC 2.0 → `synapsed` unix socket)

Source: `crates/synapse-mcp/src/main.rs`. `smx_*` tools belong to the market spinoff (cut at split). CLI flags: `-s/--sock` (default `/tmp/synapse.sock`), `--market-db`.

| Tool | Params | What it does |
|---|---|---|
| `memory_save` | `text, title?, tags?[]` | Save a memory (tags stored as `tags:` uri). Returns doc id |
| `memory_search` | `query, k=10, mode=Hybrid, embed_query=true` | Hybrid semantic+keyword search, top-k |
| `memory_recent` | `n=20` | n most recently saved memories |
| `memory_delete` | `id` | Delete a memory by id |
| `agent_observe` | `agent_id, text, project?, title?, kind=observation, tags?, source_uri?, confidence?, valid_from?, valid_until?, embed=true` | Store a scoped, typed agent observation with freshness meta (`synapse.agentdb.v1` schema) |
| `agent_search_index` | `agent_id, query, project?, limit=8, snippet_chars=240` | Compact scoped first-pass recall hits (scope-filtered + term-ranked) |
| `agent_get_observations` | `agent_id, ids[], project?, max_chars?` | Hydrate full scoped observations by id (scope-checked, optional truncation) |
| `agent_context` | `agent_id, query, project?, token_budget=800, index_k=8, full_k=3` | Token-budgeted XML context pack (index + selected full obs); reports token savings vs naive recall |
| `agent_feedback` | `agent_id, query, outcome, project?, hit_ids?, accepted=true` | Log accepted/rejected recall outcomes for learned reranking |
| `put` | `text, title?, uri?, embed?` | Low-level append |
| `search` | `q, mode?, limit?, embed_query?` | Low-level lex/vec/hybrid search |
| `merge` | `id, state[]` | Merge CRDT (yrs) state into a doc |
| `timeline` | `limit?, offset?` | Docs by timestamp descending |
| `verify` | `id, vk[]` | Verify Ed25519 signature |
| `synapse_merge` | `snapshot_path, out_path=/tmp/synapse-merged.brainpack, level=3` | CRDT merge a peer brainpack into the brain |
| `synapse_verify` | `doc_id, vk[]` | Verify Ed25519 signature by doc id |
| `smx_candles`* | `ticker, start, end, limit=500` | (market) OHLCV candles in a range |
| `smx_signal_similar`* | `ticker, date_ts, n=10` | (market) N most similar past regimes |
| `smx_pattern_stats`* | `pattern` | (market) aggregate stats for a named pattern |
| `smx_correlation`* | `tickers[], days=30` | (market) pairwise close-price correlation matrix |

\* `smx_*` → moves to **synapse-market** repo; the MCP→market dependency is removed.

### 1.3 Python API (crate `synapse-py`, module `synapse`, via maturin)

Source: `crates/synapse-py/src/lib.rs`.

| Class / fn | Methods / signature | What |
|---|---|---|
| `Brain(path)` | `put_text(text, uri?, title?)`, `search_lex(q, limit=10)`, `search_vec(embedding, limit=10)`, `search_hybrid(q, embedding, limit=10)`, `put_with_embedding(text, embedding, uri?, title?)` | Thin wrapper over core `Store`; caller supplies embeddings |
| `Synapse(path="./brain.db")` | `put(doc_id, text, metadata?)`, `search(query, k=10)`, `search_hybrid(query, embedding, k=10)`, `close()` | High-level convenience store wrapper |
| `AdaptiveRouter()` | `choose(corpus_size, latency_budget_us=0, min_recall=0.0)`, `observe(strategy, us, recall)`, `decisions()`, `posterior()` | SIMSIMD/MRL strategy picker with online posterior (Bayesian) |
| `I8Index.build(rows)` | `search(query, k=10)`, `len()`, `is_empty()`, `dim()` | Dense int8 brute-force index (SIMSIMD) |
| `F16Index.build(rows)` | `search(query, k=10)`, `len()`, `is_empty()`, `dim()`, `packed_bytes()` | Dense f16 cosine index (~50% RAM, recall≥0.99) |
| `HammingIndex.build(rows)` | `search(query, k=10)`, `len()`, `is_empty()`, `dim()` | 1-bit Hamming candidate index |
| `MultiIndex.build(rows)` | `search(query, latency_budget_us=0, min_recall=0.0, k=10)`, `len()`, `is_empty()` | I8+F16+Hamming behind AdaptiveRouter |
| `rerank(hamming_idx, i8_idx, query, k=10, candidates=80)` | fn | Two-stage Hamming candidate-gen → int8 rerank |
| `cos_f32(a, b)` / `dot_i8(a, b)` / `hamming_b8(q, db)` | fns (feature `simsimd`) | Direct SIMSIMD kernels |
| `truncate_row(v, k)` | fn | Matryoshka truncate + L2 renormalize to first k dims |

Strategy names exposed: `scalar, rayon, simsimd_f32, simsimd_i8, simsimd_hamming, mrl_simsimd, rabitq_cascade`.

### 1.4 JS / Node API (crate `synapse-js`, napi)

Source: `crates/synapse-js/src/lib.rs`.

| Class | Methods | What |
|---|---|---|
| `Synapse(path)` | `put(id, text, meta_json?)`, `search(query, limit)`, `search_hybrid(query, embedding, limit)`, `close()` | Async napi store wrapper; `SearchHit{id,uri,title,text,score}` |

### 1.5 Memory-side library crates — key public API

| Crate | pub_fns | Key public API |
|---|---|---|
| `synapse-space` | 16 | `Space::open/name/wing/store_put/search/search_reranked/search_hybrid/search_hybrid_rrf`; `Wing`/`Room` hierarchical scoping; `mcp`, `embed_bridge` modules |
| `synapse-extract` | 12 | `Extractor` trait; `RuleExtractor`, `MlxExtractor`; `ExtractedMemory`, `ExtractedRelation`; `relate_extracted`, `upsert_entity`, `run_once`, `enqueue_extraction_helper`, `ingest_and_extract`; `minimax` module |
| `synapse-rerank` | 12 | `Reranker` trait; `IdentityReranker`; `blend`; modules `cascade`, `colbert`, `factory`, `clicklog`, `lightgbm`, `onnx` (`OnnxCrossEncoder::new/new_jina_v2/from_model`) |
| `synapse-learn` | 37 | modules `bandit`, `calibrate`, `consolidate`, `drift`, `feedback`, `heat`, `rrf_tune`, `query_log`, `db`; `LearnStore` (memory-type bonus, context-query log) |
| `synapse-temporal` | 3 | `TimeRange` (`new/day_of`), `Locale`, `parse_temporal(phrase, locale)` — NL date-range parsing for time-scoped recall |
| `synapsed` | 15 | Daemon: `proto::Request` enum (see §2.2), `livequery`, `metrics`; msgpack-over-unix-socket server |
| `synapse-colbert` | 24 | Late-interaction: modules `embedder`, `kernel`, `muvera`, `quant`, `store` (kernel-only dep → could live shared) |
| `synapse-splade` | — | Neural-sparse: `block_max`, `encoder`, `index` |
| `synapse-fusion` | — | `muvera_rrf`, `full_pipeline`, `search_muvera_full`, `DenseStore`; `MuveraResult`/`MuveraLatency` |
| `synapse-multimodal` | — | `embedder`, `index`, `mime`, `storage` (cross-modal) |
| `synapse-media` | 26 | `audio_embed`, `video_embed`, `ingest`, `integrations`, `db`, `types` |
| `synapse-metal` / `synapse-embed-gpu` | — | Apple Metal / GPU embedding acceleration |

---

## 2. synapse-db — functions

### 2.1 SQL-wire surface (crate `synapsql`)

Wire: MySQL :3306 · Postgres :5432 · HTTP/gRPC :9477. Source: `crates/synapsql/src/`.

**Server entry** — `Service::new(store)` with `.with_mysql(addr)/.with_pg(addr)/.with_http(addr)` (defaults `127.0.0.1:{3306,5432,9477}`); spawns `synapse-mysql::serve`, `synapse-pg::serve`, `http::serve`. Adapters in `server/` (`mysql.rs`, `pg.rs`, `http.rs`, `brain_adapter.rs`).

**Parser/proxy layer** (`parser/`): statement fingerprinting, blake3-keyed LRU result cache (conformal write-epoch invalidation), per-connection prepared-statement cache (max 1000/conn), read/write classifier, introspection intercept (`SELECT 1`, `VERSION()`, `SHOW VARIABLES`), global QPS counter (target ≥10k single core).

**SQL extensions** (`sql_ext/`):

| Extension | Syntax | What |
|---|---|---|
| Vector distance | `... WHERE col <=> :q LIMIT k` | `VectorOp::parse/execute` — cosine-distance kNN rewrite (HNSW index scan) |
| Hybrid rank | `HYBRID_RANK(bm25_rank, vec_rank)` UDF; 3-arg `HYBRID_RANK(text, embedding, query)` planned | RRF fusion score over FTS5 + vector ranks |
| Recall guarantee | `... WITH RECALL_GUARANTEE 0.99` | `strip_recall_clause` → conformal `conformal_target` alpha |
| Time-travel | `SELECT ... AS OF '<RFC3339>'` or `AS OF LAMPORT 42` | `parse_as_of` → CRDT snapshot query |
| Graph CTE | `WITH GRAPH_TRAVERSE(start=, edge_table=, max_depth=) SELECT * FROM traverse_result` | macro → standard recursive CTE |
| EXPLAIN | `EXPLAIN [ANALYZE] <sql>` | plan builder: `<=>`→HNSW scan, `HYBRID_RANK`→Hybrid RRF, else SQLite passthrough; cols `step,op,detail,estimated_cost` |

**Binaries**: `synapsql` (server), `synapsql_bench`.

### 2.2 `synapsed` daemon protocol (DB-side service surface)

Length-prefixed msgpack over unix socket (`/tmp/synapse.sock`). `proto::Request` ops — source `crates/synapsed/src/proto.rs`:

`Ping · Put · PutBatch · Search{mode,q,limit,embed_query} · SearchScoped{...,scope_key,scope_value,candidate_limit} · Stats · Snap{out,level} · Shutdown · Merge{id,state} · Delete{id} · Timeline{limit,offset} · Verify{id,vk} · Embed{text,dim?} (server-side embed + MRL truncate) · SearchVec{embedding,limit} · Rerank{query,candidates,top_k} · SnapMerge{snapshot_path,out_path,level} · BatchSearch{queries[]} · Sql{query,params} (read-only) · Transaction{ops} (atomic all-or-nothing) · Auth{token} · UseTenant{name} (ATTACH per-tenant brain RO)`

Plus `livequery` (live/streaming query) and `metrics` (TCP metrics endpoint).

### 2.3 DB-side library crates — key public API

| Crate | Surface |
|---|---|
| `synapse-server` | HTTP/control plane; depends auth, graph, libsql, mysql, ops, pg, tune |
| `synapse-mysql` / `synapse-pg` | `serve(addr, store)` wire-protocol servers (both depend `synapse-libsql`) |
| `synapse-libsql` | libSQL/SQLite backend adapter |
| `synapse-auth` | `Role` (allows_write/allows_admin), `ApiKey`, `AuthStore` (`add_key/authenticate/require_write/require_admin/revoke/count`), `hash_key` |
| `synapse-tune` | `TuneProfile` (`safe_default/turbo_cache/financial/pragmas`), `Synchronous`/`JournalMode`/`LockingMode`, `WorkloadStats`; modules `advisor`, `bandit`, `classifier`, `drift`, `tabpfn` (auto-PRAGMA tuner) |
| `synapse-olap` | `router`, `engine` — OLAP query routing (DuckDB-style) |
| `synapse-mlx-olap` | MLX-accelerated OLAP |
| `synapse-tsdb` | time-series store + `fallback` (partial) |
| `synapse-cluster` | `Node` (`new/new_with_consensus/add_peer`), `ConsensusMode`, `PeerInfo`; modules `proto`, `raft`, `transport` |
| `synapse-raft` | Raft consensus stubs (4 structs, STUB) |
| `synapse-tier` | `ColdTier` trait, `MemoryTier`, `ObjectStoreTier` (S3/object-store cold tier) |
| `synapse-stream` | `cdc`, `cq` (continuous queries), `pubsub`, `kafka` |
| `synapse-jit` | `Value`/`Row`, modules `ir`, `schema`, `jit` — JIT query compilation (partial) |
| `synapse-iouring` | Linux io_uring LSM: `compaction`, `lsm`, `store`, `uring`, `error` (partial, linux) |
| `synapse-ring` | ring-buffer IO primitives |
| `synapse-migrate` | schema migrations |
| `synapse-ops` | operational helpers (depends core) |
| `synapse-cms` | content-management surface (depends core) |
| `synapse-license` | license validation (used by synapsed) |

---

## 3. Shared foundation API

Crates: `kernel, core, engine, ann, fts, graph, quant, spann, obs`. These live in synapse-db; synapse-memory depends on them.

### 3.1 `synapse-core` (14.6k LOC, 322 pub fns, 96 structs — the foundation)

**`Store` CRUD** (`src/db.rs`):
`Store::open(path)` · `put(req) -> id` · `put_signed(req, sk) -> id` · `put_batch(reqs) -> Vec<id>` · `get(id) -> Doc` · `delete(id) -> bool` · `search(q, SearchMode, embedding?, limit) -> Vec<Hit>` · `search_vec_exact(emb, limit) -> Vec<Hit>` · `verify(id, vk)` · `timeline(limit, offset) -> Vec<Doc>` · `stats() -> Stats`.

**Types** (`src/types.rs`): `EMBED_DIM` (1024/768/384 by feature) · `SearchMode{Lex, Vec, Hybrid}` · `PutRequest{title,uri,text,meta,embedding}` · `Doc` · `Hit{id,uri,title,text,score,meta,ts}` · `Stats`.

**Modules**: `backend` · `crdt` (yrs CRDT merge) · `db` · `error` · `federate` (`Federation`, `Addr`, peer sync) · `fresh` (`FreshMode/FreshOptions/build_fresh_report/render_fresh_context_xml`) · `shard` (k-means split, `ShardManager`) · `sign` (Ed25519 `keygen/load_signing_key/load_verifying_key/sign_bytes`) · `snap` (`export/export_signed/import/merge_packs/encrypt_pack/decrypt_pack`) · `sync` · `embed` (`Embedder::new_with_cache/embed_one`) · `embed_mlx` · `embedder_trait` · `turbo` (adaptive_router, multi_index, inmem_{f16,i8,hamming}_index, simsimd_kernels) · `matryoshka` (`truncate_row`) · `brainpack` · `obs` · `sql_fns` (`register_synapse_match` UDF) · `synx` · `sota`/`sota_ner`/`sota_pipeline` (NER/entity pipelines) · `ppr` (`personalized_pagerank`, `DEFAULT_NEIGHBOR_CAP`) · `ann` · `conformal` (conformal recall guarantee).

### 3.2 `synapse-kernel` (SIMD hot kernels — the "S0–S8" speedup tiers)

Source `crates/synapse-kernel/src/kernels/`. `prefetch<T>(ptr)`; modules `kernels`, `layouts`, `workloads`. Tier labels map to the SimSIMD benchmark stack:

| Tier | Kernel fn(s) | Notes (FACTS speedup) |
|---|---|---|
| f32 L2 | `f32_l2::l2_sq(a,b)` | scalar baseline |
| S3 int8 | `i8_dot::dot_i8 / dot_i8_scalar / dot_i8_neon` | int8 dot, 46× |
| S4 1-bit | `bin_hamming::hamming_u64(a,b)` | packed Hamming, 71× peak |
| S5 MRL-128 | (via `core::matryoshka` + simsimd) | Matryoshka 128-dim, 35× |
| S8 f16 | `f16_dot::dot_f16 / dot_f16_scalar / dot_f16_neon`, `f16_neon::dot_f16` | f16 dot, 4× |

### 3.3 `synapse-ann` (18 pub fns)

`AnnIndex` trait: `insert(id, vector)`, `search(query, k) -> SearchResults`, `len`, `save(path)`/load. `AnnError` enum. Backends: `usearch_backend`, `cascade`, `glass`.

### 3.4 `synapse-quant` (14 pub fns)

`Quantizer` trait, `QuantError`. Modules: `int8` (scalar quant), `ivf` (inverted-file), `rabitq` (RaBitQ binary quantization).

### 3.5 `synapse-graph` (49 pub fns)

`ensure_schema(conn)` · `relate(from,to,rel,weight,?)` · `neighbors(conn,node,rel?,top_k)` · `traverse(conn,start,depth,top_k_per_hop,decay,?)` · `shortest_path(conn,from,to,max_depth)` · `edge_count(conn)`. Modules: `algorithms` (`top_pagerank`, `communities`), `csr`, `cypher` (Cypher subset), `datalog`, `hippo` (HippoRAG), `live`, `sql_funcs`. `GraphError`, `SCHEMA`.

### 3.6 `synapse-fts` (6 pub fns)

`FtsIndex::new(path)` · `add(doc_id, text)` · `commit()` · `search(query, top_k) -> FtsResults` · `set_last_indexed_doc_id` / `last_indexed_doc_id`. Tantivy/FTS5-backed lexical index.

### 3.7 `synapse-engine` (4 pub fns) & `synapse-spann` (15 pub fns) & `synapse-obs`

- `engine`: `rrf_fuse_safe(a, b, k)` RRF fusion; `abi` module.
- `spann`: SPANN disk-ANN — modules `build`, `index`, `posting`, `search` (depends kernel only).
- `obs`: observability/metrics helpers.

---

## 4. Out-of-scope at split (cut / archive / vertical)

- **synapse-market** (`market`, `market-py`, `market-ts`) — finance vertical → own repo; carries the `smx_*` MCP tools (FFI `smx_query_range`, `Market::open/regime_search`, `signal_patterns`). Cut `mcp -> market` dep.
- **Cut/scaffold**: `wal` (STUB), `seg` (STUB), `e2e` (empty/rebuild), `ultra` (synapsestore dup daemon → merge into synapsed or archive), `vlog`, `lib-demo`.
