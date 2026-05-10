"""synapsql — drop-in SQLAlchemy adapter for Synapse-tuned SQLite.

Wins (vs naive SQLAlchemy + sqlite3 on M4 Max):
- 3.7× single INSERT — WAL + synchronous=NORMAL + mmap + cache_size=-64MB
- 32× bulk INSERT — group-commit batching (auto on `executemany`)
- 700× hot SELECT — 16-shard ahash AutoloadCache with epoch invalidation
- 1.85× mixed OLTP — connection-aware reuse

Source patterns: `synapse-ultra/cache.rs`, `synapsql-row/batched.rs`,
docs/wp-edition/BENCH-RESULTS-2026-05-08.md.

Usage:
    # 1-line change in CRM settings.py:
    DATABASE_URL = "synapsql:///path/to/leadflow.db"
    # ... no other code-change required.
"""
from .cache import T0Cache, CacheKey
from .connection import connect, Connection, Cursor
from .pool import ConnectionPool
from .bulk import BulkWriter

__version__ = "0.2.0"
__all__ = ["connect", "Connection", "Cursor", "T0Cache", "CacheKey",
           "ConnectionPool", "BulkWriter"]

# DBAPI 2.0 spec (PEP 249)
apilevel = "2.0"
threadsafety = 1  # threads may share module, not connections
paramstyle = "qmark"

# Re-export sqlite3 exceptions so SQLAlchemy can identify them
from sqlite3 import (
    Warning, Error, InterfaceError, DatabaseError, DataError,
    OperationalError, IntegrityError, InternalError, ProgrammingError,
    NotSupportedError,
)
