# WordPress Benchmark — Real Numbers (M4 Max, 2026-04-25)

## Setup
- SQLite `/tmp/wp-bench.db`, WAL mode, 64MB cache
- 300 `wp_options` autoload rows, 1000 `wp_posts` (publish)
- FTS5 virtual table built over `wp_posts.post_content`
- Synapse daemon queried via `synx hybrid` CLI (IPC overhead included)

## Results

| Scenario | Vanilla | FTS5 | Synapse | Gain (FTS5 vs Vanilla) | Gain (Synapse vs Vanilla) |
|---|---|---|---|---|---|
| A. autoload 300 rows (p50) | 0.095 ms | — | not impl. | — | target <0.05ms (Phase 4) |
| A. autoload 300 rows (p95) | 0.115 ms | — | — | — | — |
| B/C. search 1k posts (p50) | 0.006 ms | 0.010 ms | 45.1 ms | **0.6× (FTS5 slower)** | 7500× slower |
| B/C. search 1k posts (p95) | 0.007 ms | 0.011 ms | 62.7 ms | — | — |
| E. admin pagination (p50) | 0.102 ms | — | — | — | — |
| E. admin pagination (p95) | 0.132 ms | — | — | — | — |

## Honest Interpretation

### What these numbers mean

**At 1k posts, vanilla LIKE wins outright.**
SQLite's query planner is fast enough that a full-table scan over 1000 rows (with an index on `post_status, post_type`) takes ~6 µs. FTS5 adds tokenization overhead and is slightly *slower* at this scale (10 µs p50). This is expected — FTS5 break-even is typically ~50k+ rows.

**Synapse via `synx hybrid` CLI takes ~45 ms p50.**
This is subprocess IPC + socket round-trip + embedding overhead. On 1k posts it is not competitive with raw SQLite. Synapse wins on *relevance* (semantic search), not raw speed at small scale.

**autoload pattern: SQLite is already fast.**
95 µs to fetch all 300 autoload rows is the WP TTFB bottleneck only when PHP serialization + network stack is added. Synapse autoload-cache (target <0.5 ms warm) is not yet implemented (Phase 4 plugin).

### Where Synapse actually wins today
- **Semantic relevance**: `synx hybrid "rust web framework"` returns semantically matched results across 659+ indexed docs; LIKE returns lexical hits only.
- **Cross-content recall**: Synapse searches posts + options + custom tables in one pass; vanilla requires N LIKE queries.
- **Scale**: At 50k+ posts, FTS5 will beat LIKE, and Synapse daemon latency amortizes via keep-alive (no fork cost).

### What needs Phase 4 plugin to land
- Autoload cache (`<0.5 ms` warm read replacing PHP `alloptions` query)
- Persistent daemon socket (remove 45 ms CLI fork overhead → target 5–8 ms hybrid)
- Admin list-tables cache (pagination precomputed)

## Raw latencies (all ms)

| Metric | Value |
|---|---|
| autoload p50 | 0.095 ms |
| autoload p95 | 0.115 ms |
| LIKE search p50 | 0.006 ms |
| LIKE search p95 | 0.007 ms |
| FTS5 search p50 | 0.010 ms |
| FTS5 search p95 | 0.011 ms |
| Synapse CLI p50 | 45.1 ms |
| Synapse CLI p95 | 62.7 ms |
| pagination p50 | 0.102 ms |
| pagination p95 | 0.132 ms |
