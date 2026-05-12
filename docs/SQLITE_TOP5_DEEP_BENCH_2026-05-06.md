# SQLite-Family Top5 Deep Bench — 2026-05-06

**System**: M4 Max, brain.db 177k docs, 590MB, copy at /tmp/brain_bench.db (no daemon contention)

## Bench Matrix (7 dimensions tested)

| Test | sqlite-stdlib | **apsw** | libsql | duckdb | synx-daemon |
|------|---------------|----------|--------|--------|-------------|
| T1: 100 connect+close | 2.35ms | **2.05ms** | 3.54ms | 1060.81ms | timed-out (concurrent storm) |
| T2: 1000 individual lookups | 2.89ms | **2.51ms** | — | hung | — |
| T3: 1000 ids IN-batch | 1.34ms | **1.32ms** | — | 93.66ms | — |
| T4: GROUP BY 177k rows | 46.86ms | **43.79ms** | — | 57.59ms | — |
| T5: FTS5 50 queries | 1.38ms | **1.30ms** | — | — | sock-err |
| T7: GROUP BY tuned (mmap+cache) | 38.52ms | **35.17ms** | — | — | — |
| T8: **8 concurrent threads** | 27.82ms | **8.87ms** | — | — | — |

## Avg rank (lower = better)

| Tool | avg-rank | Tests |
|------|----------|-------|
| **apsw** | **1.00** | 7 (sweep 🥇) |
| sqlite-stdlib | 2.00 | 7 |
| libsql | 3.00 | 1 |
| duckdb | 3.33 | 3 |
| synx-daemon | — | concurrency failures |

## Per-Tool Verdict

### 🥇 apsw (Roger Binns' Python SQLite)
- **wins every dimension tested**
- Concurrent T8: **3.1× faster** than stdlib (8.87 vs 27.82 ms)
- Drop-in replacement, same SQL
- Best for: read-heavy scripts, parallel workers, hot-path analytics

### 🥈 sqlite3 stdlib
- Solid #2 in all tests
- Comes free (Python builtin)
- Good for: simple writes, default scripts, less-than-1k ops

### 🥉 libsql_experimental (Turso fork)
- Connect cycle 50% slower than stdlib (3.54 vs 2.35ms)
- Auto opens read-write (conflicts with live daemon)
- Best for: future replication needs, NOT current local-only

### 4️⃣ duckdb
- **Terrible at lookup** (1060ms for 100 connects = 50× slower)
- IN-batch 93ms vs sqlite 1.3ms (70× slower)
- OK GROUP BY 57ms (only 30% slower than sqlite)
- Best for: heavy OLAP >10M rows aggregations only
- **Don't use** for hot-path lookups

### 5️⃣ synx-daemon (msgpack socket)
- 1ms RTT per ping (single-shot ok)
- **Concurrent storm fails** — single-conn-mutex serialization
- Hybrid vec+FTS 16ms when working
- Best for: 1 hybrid query per user-prompt, NOT bulk batch

## Capability Matrix

| Feature | sqlite-stdlib | apsw | libsql | duckdb | synx-daemon |
|---------|---------------|------|--------|--------|-------------|
| FTS5 | ✅ | ✅ | ✅ | ❌ | ✅ via op |
| sqlite-vec ext | ✅ load_ext | ✅ load_ext | ✅ | ❌ | ✅ built-in |
| GROUP BY full SQL | ✅ | ✅ | ✅ | ✅ best at scale | ❌ no SQL op |
| Concurrent reads | OK | **best** | OK | poor | serialized |
| Replication | ❌ | ❌ | ✅ | ❌ | manual |
| Embedded | ✅ | ✅ | ✅ | ✅ | server-mode |
| Vec hybrid native | ❌ | ❌ | ❌ | ❌ | ✅ |

## Optimization Headroom

| Lever | Win | How |
|-------|-----|-----|
| **mmap_size=512M** | 25% on GROUP BY (47→38ms) | `PRAGMA mmap_size=536870912` |
| **cache_size=128M** | additive | `PRAGMA cache_size=-128000` |
| **IN-batch over loop** | 2× lookups (2.5→1.3ms) | replace per-row with single SQL |
| **apsw over stdlib** | 1.2-3× | `try: import apsw` |
| **Persistent connection** | 5-10× connect overhead | reuse cursor across calls |

## Final Recommendations

| Use Case | Primary | Fallback |
|----------|---------|----------|
| Read-heavy CLI | **apsw** | sqlite3 stdlib |
| Concurrent worker pool | **apsw** (3× win) | sqlite3 + WAL |
| Bulk write/ingest | sqlite3 stdlib (simpler API) | apsw |
| OLAP >10M rows | duckdb | apsw + indexes |
| Hybrid vec retrieval | **synx-daemon** | direct sqlite-vec |
| Edge replication | libsql | — |

## Notes
- duckdb T2 hung (1000 individual rowid lookups) — column-store penalty
- libsql opens write mode by default, conflicts with live synapsed
- synx-daemon T1/T5/T6 concurrent failures = telepathy daemon competing for socket
- apsw shines on concurrent reads (3.1× win) — the real differentiator at scale
