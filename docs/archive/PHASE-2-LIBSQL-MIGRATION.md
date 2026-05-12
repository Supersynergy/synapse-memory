# PHASE-2: libSQL Backend Migration

Date: 2026-04-25 · Owner: Team Delta δ1/δ2/δ3
Source: subagent a181e241067da2cd3 · Verdict: ⚠️ PAUSE (see Day-1-2 corrective sprint below)

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

## ABI Compat Test — RESULT: ❌ FAIL (2026-05-05, libsql 0.9.30)

**Verdict: PAUSE**

Prior GO verdict (2026-04-24) was from a `/tmp/libsql-abi-test/` ad-hoc run using libsql = **0.6.0**.
Committed example binary using libsql = **0.9.30** (current workspace dep) **fails**.

### Root Cause

`libsql 0.9.x` introduces `libsql-rusqlite` as an internal dependency of `libsql-sys`.
`libsql-rusqlite` calls `sqlite3_config(SQLITE_CONFIG_MULTITHREAD)` + `sqlite3_initialize()`
on the shared `libsql-ffi` bundled SQLite during its own init.

When `libsql::local::Database::new()` then calls `sqlite3_config(SQLITE_CONFIG_SERIALIZED)`,
SQLite is already initialized → returns `SQLITE_MISUSE (21)` → panic.

Both sub-crates share the **same** bundled SQLite instance. Threading config cannot be set
after `sqlite3_initialize()`. This is an upstream conflict within libsql 0.9.x itself.

### What Works
- libsql **0.6.0** (no `libsql-rusqlite` dep): FTS5 + sqlite-vec PASS ✅
- libsql **0.9.30** (current): threading init conflict, panics before any SQL runs ❌

### Example Binary
`crates/synapse-core/examples/libsql_fts5_abi_check.rs`
Run: `cargo run --example libsql_fts5_abi_check -p synapse-core --features backend-libsql`

### Resolution Options
1. **Downgrade to libsql 0.6.x** — loses async API improvements, but ABI-stable
2. **Wait for upstream fix** — track https://github.com/libsql/libsql/issues
3. **Out-of-process libsql** — run libsql in a sidecar process, IPC via stdio/unix socket
4. **Stay on rusqlite** — default path unaffected; `backend-libsql` feature stays gated

### Prior PASS test (libsql 0.6.0, still valid for that version)
```rust
// libsql 0.6.0 — CORRECT pattern
unsafe {
    libsql::ffi::sqlite3_auto_extension(Some(
        std::mem::transmute::<*const (), unsafe extern "C" fn(*mut libsql::ffi::sqlite3, *mut *const i8, *const libsql::ffi::sqlite3_api_routines) -> i32>(
            sqlite_vec::sqlite3_vec_init as *const ()
        )
    ));
}
let db = libsql::Builder::new_local("/tmp/test.db").build().await?;
let conn = db.connect()?;
conn.execute("CREATE VIRTUAL TABLE v USING vec0(id INTEGER PRIMARY KEY, e FLOAT[384])", ()).await?;
conn.execute("CREATE VIRTUAL TABLE f USING fts5(content)", ()).await?;
println!("PASS: FTS5 + vec0 both work on libsql 0.6.0");
```

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

## Recommendation: ⚠️ PAUSE

libsql 0.9.x threading conflict blocks backend switch. Default `backend-rusqlite` is stable and unaffected.
Next action: choose resolution option above (downgrade / out-of-process / stay-on-rusqlite).
BEGIN CONCURRENT projected gain (5.7×) is still the target if resolved.
