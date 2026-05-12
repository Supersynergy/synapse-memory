# opensrv-mysql Migration Plan

**Status**: Phase 1 scaffolded (2026-04-25)  
**Crate**: `crates/synapse-mysql`  
**Feature flag**: `async-proxy` (default OFF)  
**Binary**: `synapse-mysql-async` on `:13310`

## Phase 1 — Scaffold (DONE)

- `opensrv-mysql v0.7` added as optional dep
- `server_async.rs`: `AsyncMysqlShim` impl — SELECT 1, @@version, ping → OK
- `main_async.rs`: tokio TcpListener, per-connection spawn, split r/w streams
- Build: `cargo build --features async-proxy -p synapse-mysql` → green

## Phase 2 — Full read-path port (~4–6h)

Requires porting `execute_read_query` from `server.rs` into async context:

1. Move `open_read_conn()` to be `async fn` (or wrap in `tokio::task::spawn_blocking`)
2. Pipe SQL through `rewrite::rewrite()` before executing
3. Return proper column metadata + row data via `QueryResultWriter`
4. Support `on_prepare` / `on_execute` with parameter binding
5. Wire in `Acl::check_auth()` on `on_init` / handshake

## Phase 3 — Write-path + coalescing (~2–3h)

1. Port `CoalesceBuffer` (wp_options write coalescing) to async
2. Use `tokio::time::sleep` instead of `std::thread::sleep`
3. Ensure `EXCLUSIVE` SQLite write lock is held only during flush
4. Benchmark: target ≥ current sync throughput at same wp_options load

## Phase 4 — Full parity + port cutover (~1–2h)

1. Prepared-statement fingerprint cache (`FpCache`) → async-safe (DashMap or RwLock)
2. Enable `async-proxy` in default features
3. Move `:3306` default to `synapse-mysql-async`, shift sync server to `:13311`
4. Remove `synapse-mysql` sync binary after 30d stabilisation
5. CI: add smoke test — `mysql -h 127.0.0.1 -P 13310 -e "SELECT 1"` → pass

## Blockers / Notes

- `opensrv-mysql` does not support TLS in the current scope; use stunnel/nginx for production TLS
- SQLite connection (`rusqlite`) is `!Send` — all DB calls in Phase 2 must use `spawn_blocking`
- `msql-srv` (sync) stays as fallback until Phase 4 cutover is validated
