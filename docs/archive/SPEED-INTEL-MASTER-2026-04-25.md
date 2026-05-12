# SPEED-INTEL Master Index — Ultrathink Orchestration

Date: 2026-04-25 (late)
Goal: 10-100× MariaDB/Percona via every architecture pattern that works.
Mode: ultrathink coordination of 3 parallel research subagents + ghgrep verification.

## Live Subagent Roster

| Researcher | Mission | Status |
|---|---|---|
| **Speed-A (a1a4be03)** | Top-10 OLTP DBs + 15 patterns + 5 Synapse adoptions | ✅ DONE |
| **Speed-B (ae368c7f)** | Bulk ingest 1M docs/sec | RUNNING |
| **Speed-C (acdd9903)** | Concurrent ops 100k QPS @ 64t | RUNNING |
| **Sec (ada7bd01)** | Fix rustls-webpki CVEs | RUNNING |

## Speed-A Verified Findings (just landed)

### Top-3 Synapse-specific adoptions ranked
1. **Async Group Commit** (FastFS pattern) — coalesce 100s writes into 1 fsync → **3-5× throughput** (3-5 days)
2. **Shard-per-core Read Path** — tokio::task_local + per-CPU arena → **2-3× latency** (5-7 days)
3. **Lockless HashMap** (dashmap v5 + parking_lot v1) — replace RwLock<BTreeMap> → **4× concurrent reads** (2-3 days)

### Synapse architecture insight (Speed-A finding)
Current 23µs query is MEMORY-layer optimal. To reach 100k OLTP OPS, shift focus from index to **write concurrency + WAL throughput**.

**Realistic target: 50-100k OPS in 90 days** (matching MariaDB on bare-metal single-node).

## Compound Roadmap (post Speed-A)

### Phase 8 (new) — Write Throughput Sprint
- Day 1-3: dashmap replace RwLock (4× concurrent)
- Day 4-7: Async group commit (3-5× write)
- Day 7-12: Thread-per-core read path (2-3× latency)
- **Compound:** 1078 QPS → ~25-50k OPS @ 8t

### Combined with libSQL BEGIN CONCURRENT (Phase 2)
- libSQL: +4× write-mix
- + Group commit: +3-5× sustained writes
- + Shard-per-core: +2-3× per-thread parallelism
- + dashmap: +4× concurrent read overlap
- **Total potential: 1078 → 100-300k OPS** (matching MariaDB)

### Stretch — 1M OPS architecture
Per Speed-A: needs custom storage engine (LSM + io_uring + DPDK for network).
- Likely Phase 12+ (Q3 2026)
- ScyllaDB cites 10M ops/s on 24-core; M4 Max has 16 cores → ~6M theoretical ceiling

## Awaiting Speed-B + Speed-C Outputs

### Speed-B will deliver
- Top-10 bulk ingest patterns (current 6µs/doc → 1M docs/sec)
- ScyllaDB sstableloader + Cassandra bulk + ClickHouse direct LSM patterns
- Tier-1 (100k/sec), Tier-2 (1M/sec), Tier-3 (10M/sec sustained) roadmap
- Embed bottleneck (fastembed 30ms = 33 docs/sec ceiling per thread)

### Speed-C will deliver
- ScyllaDB shard-per-core feasibility on M4 Max (12 P-cores + 4 E-cores)
- Specific code refactor for synapse-mysql-async (1078 → 50k QPS)
- Trade-off matrix: thread-per-core vs tokio multithread vs spawn_blocking
- 14-day plan: profile → refactor → re-bench

## Critical Bottleneck (current understanding)

```
synapse-mysql-async pipeline @ 8t:
  TCP accept → tokio task → spawn_blocking → rusqlite Connection
                                  ↑
                                  HERE — 1 OS thread per query
                                  thread pool default = num_cpus
                                  serial fsync on writes
                                  no group commit
```

Solutions ranked by ROI:
| Fix | Effort | Gain | Phase |
|---|---:|---:|:-:|
| Connection pool 64+ rusqlite | 1d | +3× | 8 |
| Async group commit | 3-5d | +3-5× | 8 |
| dashmap replace RwLock | 2-3d | +4× concurrent | 8 |
| Shard-per-core read | 5-7d | +2-3× | 8 |
| libSQL BEGIN CONCURRENT | 7d (Phase 2) | +4× write | 2 |
| io_uring (Linux only) | 14d | +5-10× writes | 12+ |
| Custom LSM | 60d | +50× writes | future |

## ghgrep evidence (Speed-A cited)

Will be appended once Speed-B + Speed-C deliver verified repo:file:line refs.

## Meta-Insight (ultrathink synthesis)

The Synapse advantage isn't going to come from raw MySQL parity — that's a 6-12 month custom-storage-engine project. The advantage comes from:

1. **Hybrid SQL + Vector + FTS in one binary** (no other engine ships this)
2. **Cold start < 100ms** (MariaDB 5-30s)
3. **Embedded library mode 5µs reads at scale** (no SQL DB has this)
4. **AI-built-in SQL functions** (synapse_match etc — Phase 5)
5. **Adaptive ML routing** (CatBoost router — Phase 2B)

Position Synapse NOT as "faster MariaDB" but as "the database for AI-era apps where MariaDB simply can't compete on capability". Speed parity (50-100k OPS) is necessary but not sufficient. The compound capability moat is the 10-100× story.

## Status: 1/3 research agents back. 2 running. 1 sec fix running. Master synthesis pending all 4.
