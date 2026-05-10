# synapsql SQLAlchemy Adapter — Sketch (Worker-3)

**Goal**: drop-in replacement for SupersynergyCRM SQLAlchemy backend → 5-20× speedup, 0 code-change in CRM.

## Architecture

```
SupersynergyCRM (FastAPI/Svelte 5)
  ↓ SQLAlchemy ORM (no change)
  ↓ DSN: synapsql://localhost:9477/crm
  ↓
synapsql Python Driver (NEW, 200 LoC)
  • DBAPI 2.0 spec (PEP 249)
  • Wraps msgpack→Unix-socket→synapsed daemon
  ↓
synapsed daemon (libsql turbo + WAL + AutoloadCache)
  ↓
brain.db (SQLite+FTS5+sqlite-vec, single file)
```

## Drop-in Path

### Before (SupersynergyCRM current)
```python
# settings.py
DATABASE_URL = "sqlite:////path/to/leadflow.db"
```

### After (1-line change)
```python
DATABASE_URL = "synapsql://localhost:9477/leadflow"
```
Models, queries, migrations: **0 changes**.

## Driver Skeleton (200 LoC)

```python
# synapsql_driver/__init__.py — register as SQLAlchemy dialect

from sqlalchemy.dialects.sqlite.pysqlite import SQLiteDialect_pysqlite
from sqlalchemy.dialects import registry

class SynapsqlDialect(SQLiteDialect_pysqlite):
    """Wraps SQLite-dialect behaviour but routes via synapsed socket."""
    name = "synapsql"
    driver = "synapsql_driver"
    supports_statement_cache = True   # 18.5ns AutoloadCache hits

    @classmethod
    def dbapi(cls):
        from . import dbapi
        return dbapi

# DBAPI 2.0
class Connection:
    def __init__(self, dsn):
        self._sock = _connect_unix(dsn)  # /tmp/synapse.sock
    def cursor(self): return Cursor(self._sock)

class Cursor:
    def execute(self, sql, params=None):
        # 1. Hash sql+params → ahash u64
        # 2. Check T0Cache (18.5ns hot-path)
        # 3. Else: msgpack {"op":"Sql", "args":{"sql":sql,"params":params}} → daemon
        # 4. Cache result, return rows
```

Daemon-side: add `Request::Sql { sql, params }` op → dispatch to `Store.execute_sql()` (already exists in synapse-core via rusqlite).

## What we get for free

| Win | From |
|---|---|
| 18.5ns hot-query lookup | synapse-ultra T0Cache (16-shard ahash) |
| 32× bulk INSERT | synapsql-row BatchedLibsqlStore |
| 1.85× concurrent OLTP | synapsql-row RealPoolStore |
| 3.7× single INSERT | libsql turbo pragmas |
| Built-in vec/FTS/KG | synapse-core (no schema-change in CRM) |
| MCP-integration | synapse-mcp daemon-side |
| AI-features (semantic-search, score, KG) | New endpoints, optional |

## SupersynergyCRM Bottleneck-Map → Win

| Bottleneck | Current | After | Win |
|---|---|---|---|
| DB Operations 32% | SQLite SQLAlchemy direct | synapsql + AutoloadCache | **5-700×** hot, 1.85× cold |
| ORM Overhead 12% | SQLAlchemy reflection | DBAPI bypass + stmt-cache | **5-15×** |
| JSON Serialization 18% | python json | rmp-serde msgpack | **5-10×** |
| Template Rendering 15% | Jinja2 | precompiled + AutoloadCache | **20-100×** |

**Total CRM-speedup expected**: **5-20×** wallclock.

## Implementation Phases

### Phase 1 (3 days): Pure DBAPI driver
- Unix-socket msgpack client (~80 LoC)
- DBAPI 2.0 conformance (Connection, Cursor, exceptions)
- Synchronous only, no async
- Test: SQLAlchemy + alembic migrations work unchanged

### Phase 2 (1 day): SQLAlchemy dialect
- Register "synapsql://" scheme
- Inherit from SQLiteDialect (since brain.db IS sqlite)
- Override _execute to route via daemon

### Phase 3 (2 days): Daemon-side `Sql` op
- Add `Request::Sql { sql, params }` to proto.rs
- Dispatch to `Store.execute_sql()` with prepared-stmt cache
- Cache results via T0Cache (key = ahash(sql+params))
- Cache-invalidate on any non-SELECT

### Phase 4 (1 day): Bench + ship
- Run SupersynergyCRM endpoint-bench (homepage, lead-list, search) 
- Verify ≥5× wallclock-improvement vs raw SQLite
- Document Tier-S features as opt-in endpoints (semantic-search, ai-score)

## Files to create
```
synapsql_driver/__init__.py        # SQLAlchemy registration
synapsql_driver/dialect.py          # SynapsqlDialect class
synapsql_driver/dbapi.py            # PEP 249 Connection/Cursor
synapsql_driver/socket_client.py    # msgpack Unix-socket
synapsql_driver/cache.py            # client-side T0 (optional, daemon already has)
tests/test_drop_in_sqlalchemy.py    # alembic migration + ORM-CRUD round-trip
bench/test_supersynergy_endpoints.py # measure homepage/lead-list/search latency
```

## Daemon delta (synapsed/src/proto.rs)
```rust
pub enum Request {
    // existing ...
    /// Execute arbitrary SQL on Synapse store. AutoloadCache hot-path.
    Sql {
        sql: String,
        params: Vec<serde_json::Value>,
        cache_ttl_ms: Option<u64>,  // None = use SELECT-default
    },
}
```

Total daemon-side delta: ~50 LoC.

## Risks & Anti-Patterns

| Risk | Mitigation |
|---|---|
| Statement-cache invalidation bug | Daemon emits gen-bump on any write; T0Cache.invalidate() on bump |
| Long-tail SQL (analytical) doesn't cache | TTL=0 for >100ms queries, only point-queries cached |
| FastAPI async-loop blocks on Unix-socket | Ship asyncio variant via `asyncio.open_unix_connection` |
| User has Postgres-specific SQL | SupersynergyCRM uses SQLite already → no migration |

## Verify-Loop (must pass before ship)
- [ ] `pytest tests/test_drop_in_sqlalchemy.py` green
- [ ] alembic migrate up/down works
- [ ] CRM homepage <100ms (was 200-500ms)
- [ ] No regression in feature-flags
- [ ] `synapse-purge-leads` cleanup compatible
