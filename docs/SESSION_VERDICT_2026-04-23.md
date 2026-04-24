# Session Verdict — Synapse v2 — 2026-04-23

**One page. Honest.**

---

## What was built this session

| Area | Work done |
|------|-----------|
| WP 3-way bench | Synapse proxy vs MySQL vs SQLite for WordPress-style queries. Sysbench reachable after SSL/user fix. |
| YCSB fair 8t vs 8t | go-ycsb against Synapse proxy (port 3309) at 8 threads, ops=10k |
| Library-mode | Scale ladder 1k/10k/100k/1M + concurrent reader test |
| Sysbench | Point-select 1t/8t numbers against Synapse proxy |

---

## Numbers

### YCSB — 8-thread fair comparison (Synapse proxy vs MySQL direct)

| Workload | Synapse 8t OPS | MySQL 8t OPS | Ratio |
|----------|---------------|-------------|-------|
| A (50r/50u) | 2,469 | 5,520 | 45% |
| B (95r/5u) | 2,495 | 27,091 | 9% |
| C (100r) | 2,504 | 63,402 | 4% |
| F (RMW) | 2,497 | 12,333 | 20% |

Synapse proxy = MySQL wire → SQLite. Every op pays ~3ms IPC+serde. MySQL = direct.

### Library-mode scale ladder

| Docs | put_µs | lex_µs | vec_µs |
|------|--------|--------|--------|
| 1k | 59 | 68 | 6.5 |
| 10k | 61 | 238 | 6.1 |
| 100k | 94 | 2,117 | 6.2 |
| 1M | 119 | 23,989 | 6.6 |

Vec search **constant at 6µs** across all scales (sqlite-vec SIMD compute-bound).

Concurrent readers @ 100k: 4t=9µs, 8t=7.9µs, 16t=7.6µs per op (mutex, low contention).

### Sysbench (port 13308, Synapse proxy)

| Workload | Threads | QPS |
|----------|---------|-----|
| oltp_point_select | 1 | 4,817 |
| oltp_point_select | 8 | 16,258 |

### 100M arithmetic projection

Defended, not measured. Claim: at 6µs vec search × 100M ops = 600s wall-clock on 1 thread. With 16 concurrent readers and amortized batching this projects to ~40-60s. **Not a benchmark result — a scaling projection.**

---

## What Synapse WINS today

- **Library-mode speed**: 6µs vec search vs 3,450µs MCP round-trip = **569×** speedup
- **Zero-infra**: single SQLite file, no daemon required in library mode
- **Sysbench point-select**: 4,817 QPS at 1t, 16,258 at 8t (low-latency proxy path works)
- **Wire compatibility**: go-ycsb A/B/C/F all complete 0 errors at 8 threads

Source docs: `docs/LIBRARY_MODE_DEMO_2026-04-23.md`, `docs/BENCH_SUITES_RESULTS_2026-04-23.md`

---

## What Synapse LOSES

| Gap | Detail |
|-----|--------|
| Raw throughput vs MySQL | 4–45% of MySQL OPS in proxy mode (YCSB 8t) |
| FTS5 scan at 1M docs | 24ms lex search — needs FTS5 trigram or pre-filter index |
| Multi-writer | SQLite WAL = 1 writer. Concurrent writes serialize. |
| ann-benchmarks | No sqlite-vec Python adapter for standard ANN comparison |
| YCSB at scale | Tested at 1k records; 1M record YCSB not yet run |

**Next PRs**:
- **PR-A2**: async batch-put API — amortize WAL flush, fix throughput cliff at 100k+
- **PR-G1**: FTS5 trigram index — fix lex search linear scan at 1M docs
- **multi-writer**: WAL + connection pool with write queue for concurrent ingestion

---

## 20-Personas Score

Prior: 6/20 (before YCSB fix, library-mode, and sysbench).

After this session:
- Embedded Rust devs: library-mode direct API → now viable ✓
- Agent framework authors: 6µs vec at 1M docs → now viable ✓
- WordPress/MySQL-wire users: YCSB 0 errors at 8t → now viable ✓
- Knowledge base tools: sysbench 16k QPS → now viable ✓

**Revised estimate: ~9–10/20** — gains in embedded, agent, and wire-compat personas. Still weak for high-throughput OLTP, multi-writer, and ANN benchmark personas.

---

## Honest marketing claim

> "Synapse library-mode: 6µs vector search, 94µs put — no daemon, no IPC, single SQLite file. 569× faster than MCP round-trip."

**Anti-claim** (do not use without caveat):
> "Synapse outperforms MySQL" — false in proxy mode. Proxy OPS is 4–45% of MySQL direct. Library-mode is not a MySQL replacement; it is an embedded memory layer.
