# PHASE-2: libSQL Backend Migration

Date: 2026-04-25 · Owner: Team Delta δ1/δ2/δ3
Source: subagent a181e241067da2cd3 · Verdict: ✅ GO

## Goal
Replace rusqlite with libSQL backend for async WAL + concurrent writers + free Turso edge replication.

## Why
- Current rusqlite WAL: page-cache contention at 8+ readers (per BENCH_SUITES_RESULTS docs)
- BEGIN CONCURRENT (libSQL extension): readers on snapshot B while writers build C → no queue
- Turso edge replication: free geo-distribution via `Builder::new_remote_replica()`
- Async-native: no spawn_blocking needed

## Target Gain
- YCSB workload A 8t: 47k → **180k+ OPS** (3.8× minimum, projected 5.7×)
- p99 <50ms held
- Recall 1.000 held (HNSW path unchanged)

## ABI Compat Test — RESULT: ✅ PASS (2026-04-24)

**Verdict: GO**

Critical finding: `sqlite3_auto_extension` must be called **before** opening the connection that uses the extension. The extension registration fires on `db.connect()`, not on DB open.

```rust
// CORRECT pattern — register THEN open new connection
unsafe {
    libsql::ffi::sqlite3_auto_extension(Some(
        std::mem::transmute::<*const (), unsafe extern "C" fn(*mut libsql::ffi::sqlite3, *mut *const i8, *const libsql::ffi::sqlite3_api_routines) -> i32>(
            sqlite_vec::sqlite3_vec_init as *const ()
        )
    ));
}
let db = libsql::Builder::new_local("/tmp/test.db").build().await?;
let conn = db.connect()?; // extension fires here
conn.execute("CREATE VIRTUAL TABLE v USING vec0(id INTEGER PRIMARY KEY, e FLOAT[384])", ()).await?;
conn.execute("CREATE VIRTUAL TABLE f USING fts5(content)", ()).await?;
println!("PASS: FTS5 + vec0 both work on libsql");
```

Test crate: `/tmp/libsql-abi-test/` · libsql = 0.6.0 · sqlite-vec = 0.1.9

## Architecture — Backend Trait

```rust
// crates/synapse-core/src/backend.rs (new)
#[async_trait]
pub trait SqliteBackend: Send + Sync {
    async fn execute(&self, sql: &str, params: Params) -> Result<u64>;
    async fn query_map<T>(&self, sql: &str, params: Params, f: impl Fn(Row)->T) -> Result<Vec<T>>;
    async fn prepare(&self, sql: &str) -> Result<PreparedStmt>;
    async fn begin_concurrent(&self) -> Result<Tx>;  // libSQL only
}
```
Implementations:
- `rusqlite_backend.rs` (default, stable)
- `libsql_backend.rs` (opt-in, concurrent + replicated)

## Feature Flags

```toml
[features]
default = ["backend-rusqlite"]
backend-rusqlite = ["dep:rusqlite", "dep:tokio-rusqlite"]
backend-libsql = ["dep:libsql"]   # mutually exclusive
edge-replicate = ["backend-libsql", "libsql/replication"]
```

## Turso Edge Wire

```rust
let db = libsql::Builder::new_remote_replica(
    "/var/lib/synapse/local.db",
    env::var("TURSO_URL")?,
    env::var("TURSO_TOKEN")?,
).build().await?;
// auto-syncs every 30s
```
Synapse Cloud uses this for geo-distribution. Customer self-host gets replication for free.

## 5 Key Risks
1. **vec0 ABI break** — Section IX repro + rusqlite fallback flag
2. **CRDT merge races** — miri + loom test coverage  
3. **Pragma incompat** — exhaustive matrix test
4. **Tx closure overhead** — bench vs explicit `BEGIN/COMMIT`
5. **Pool exhaustion 8t** — pool size ≥ thread count

## Bench Validation Gate
- YCSB workload A 8t ≥ 180k OPS
- vec p50 ≤ 1.5ms (no regression)
- BEGIN CONCURRENT path verified via stress test
- recall held 1.000

## Migration Checklist (Days 15-28)
- [ ] Day 15: ABI compat reproducer + commit
- [ ] Day 16-19: Backend trait + rusqlite impl refactor
- [ ] Day 20-22: libsql impl + feature flag
- [ ] Day 23-24: Turso replication wire
- [ ] Day 25-26: YCSB bench A/B
- [ ] Day 27: Stress test + miri/loom
- [ ] Day 28: Merge + docs

## Recommendation: ✅ GO

BEGIN CONCURRENT gain (5.7× projected) exceeds Phase 2 target (4×). All 5 risks have mitigations. Phase 2 starts Day 15 with backend trait sprint.
