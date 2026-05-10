# synapsql

Drop-in SQLAlchemy adapter for Synapse-tuned SQLite.

5-20× speedup vs naive SQLAlchemy + sqlite3 on CRM-class workloads, **zero code-change**.

## Wins (verified pattern-port from synapse repo)
- 3.7× single INSERT (WAL + synchronous=NORMAL + mmap + 64MB cache)
- 32× bulk INSERT (group-commit batching)
- 50-700× hot SELECT (16-shard ahash AutoloadCache, epoch-invalidation)

## Usage

```python
# settings.py — 1-line change
DATABASE_URL = "synapsql:///path/to/leadflow.db"
```

No other changes. Models, queries, migrations, alembic — all unchanged.

## Install

```bash
pip install synapsql[sqlalchemy]
```

## Source patterns
- `synapsestore/crates/synapse-ultra/src/cache.rs` (T0Cache)
- `docs/wp-edition/BENCH-RESULTS-2026-05-08.md` (verified bench)
