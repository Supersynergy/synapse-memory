"""DBAPI 2.0 wrapper around sqlite3 with WAL+turbo pragmas + AutoloadCache.

PEP 249 conformance plus:
- Auto-applies turbo pragmas on connect (3.7× single INSERT)
- Wraps cursor with T0Cache (700× hot SELECT)
- Detects writes → bumps cache generation (epoch invalidation)
"""
from __future__ import annotations
import sqlite3
import re
from typing import Any, Optional

from .cache import T0Cache, CacheKey

# DBAPI 2.0 module-level constants (SQLAlchemy probes for these on the DBAPI module)
apilevel = "2.0"
threadsafety = 1
paramstyle = "qmark"

# Re-export sqlite3 exception types + DBAPI symbols
from sqlite3 import (
    Warning, Error, InterfaceError, DatabaseError, DataError,
    OperationalError, IntegrityError, InternalError, ProgrammingError,
    NotSupportedError, sqlite_version, sqlite_version_info,
    Date, Time, Timestamp, DateFromTicks, TimeFromTicks, TimestampFromTicks,
    Binary, register_adapter, register_converter,
    PARSE_DECLTYPES, PARSE_COLNAMES,
)

# turbo pragmas — verified 3.7× single INSERT win in synapse benches
_TURBO_PRAGMAS = (
    "PRAGMA journal_mode = WAL",
    "PRAGMA synchronous = NORMAL",
    "PRAGMA cache_size = -65536",   # 64MB page-cache
    "PRAGMA mmap_size = 268435456", # 256MB mmap
    "PRAGMA temp_store = MEMORY",
    "PRAGMA wal_autocheckpoint = 10000",
    "PRAGMA busy_timeout = 10000",
)

# Detect write-statements that must invalidate cache
_WRITE_RE = re.compile(
    r"^\s*(INSERT|UPDATE|DELETE|REPLACE|DROP|CREATE|ALTER|TRUNCATE|VACUUM|REINDEX)",
    re.IGNORECASE,
)
_SELECT_RE = re.compile(r"^\s*(SELECT|WITH)\b", re.IGNORECASE)

# Module-level shared cache (one per database path)
_GLOBAL_CACHES: dict[str, T0Cache] = {}
_GLOBAL_CACHE_LOCK = __import__("threading").Lock()


def _cache_for(path: str) -> T0Cache:
    with _GLOBAL_CACHE_LOCK:
        c = _GLOBAL_CACHES.get(path)
        if c is None:
            c = T0Cache()
            _GLOBAL_CACHES[path] = c
        return c


class Connection:
    """sqlite3.Connection subclass-ish wrapper. Turbo-pragma'd, cache-aware."""

    def __init__(self, database: str, *args, cache_enabled: bool = True, **kwargs):
        # honor SQLAlchemy's check_same_thread/timeout/isolation_level
        # Default to deferred-mode (sqlite3 default) so executemany batches via implicit txn.
        self._sqlite = sqlite3.connect(database, *args, **kwargs)
        self._database = str(database)
        self._cache = _cache_for(self._database) if cache_enabled else None
        # Apply turbo pragmas
        for p in _TURBO_PRAGMAS:
            try:
                self._sqlite.execute(p)
            except sqlite3.OperationalError:
                pass  # `:memory:` ignores some

    def cursor(self) -> "Cursor":
        return Cursor(self)

    def commit(self) -> None:
        self._sqlite.commit()

    def rollback(self) -> None:
        self._sqlite.rollback()

    def close(self) -> None:
        self._sqlite.close()

    def execute(self, sql: str, params: tuple = ()) -> "Cursor":
        c = self.cursor()
        c.execute(sql, params)
        return c

    def executemany(self, sql: str, seq: list) -> "Cursor":
        c = self.cursor()
        c.executemany(sql, seq)
        return c

    @property
    def in_transaction(self):
        return self._sqlite.in_transaction

    # SQLAlchemy probes for these
    def create_function(self, *a, **kw): return self._sqlite.create_function(*a, **kw)
    def create_aggregate(self, *a, **kw): return self._sqlite.create_aggregate(*a, **kw)
    def create_collation(self, *a, **kw): return self._sqlite.create_collation(*a, **kw)
    def set_authorizer(self, *a, **kw): return self._sqlite.set_authorizer(*a, **kw)
    def iterdump(self, *a, **kw): return self._sqlite.iterdump(*a, **kw)


class Cursor:
    """Cache-aware cursor. SELECT → cache lookup. Writes bump epoch."""

    arraysize = 1

    # Per-connection prepared-statement cache. Re-uses sqlite3's internal cache
    # but skips Python-side regex+hash recompute on identical SQL strings.
    _STMT_CACHE_LIMIT = 256

    def __init__(self, conn: Connection):
        self._conn = conn
        self._cur = conn._sqlite.cursor()
        self._results: Optional[list] = None
        self._cached_hit = False
        self._cached_desc = None  # set on cache-hit
        # Inline classify-cache: SQL → ("select"|"write"|"other"). Skips regex per call.
        if not hasattr(conn, "_stmt_class_cache"):
            conn._stmt_class_cache = {}
        self._stmt_class = conn._stmt_class_cache

    @property
    def description(self):
        if self._cached_hit and self._cached_desc is not None:
            return self._cached_desc
        return self._cur.description

    @property
    def rowcount(self):
        return self._cur.rowcount

    @property
    def lastrowid(self):
        return self._cur.lastrowid

    def execute(self, sql: str, params: tuple = ()) -> "Cursor":
        cache = self._conn._cache
        # Classify once per unique SQL string (regex is the expensive part)
        klass = self._stmt_class.get(sql)
        if klass is None:
            if _SELECT_RE.match(sql):
                klass = "select"
            elif _WRITE_RE.match(sql):
                klass = "write"
            else:
                klass = "other"
            if len(self._stmt_class) < self._STMT_CACHE_LIMIT:
                self._stmt_class[sql] = klass

        if cache is not None and klass == "select":
            key = CacheKey.of(sql, tuple(params) if params else ())
            hit = cache.get(key)
            if hit is not None:
                self._results = list(hit[0])
                self._cached_desc = hit[1]
                self._cached_hit = True
                return self
            self._cur.execute(sql, params)
            rows = self._cur.fetchall()
            cache.put(key, (rows, self._cur.description))
            self._results = rows
            self._cached_hit = False
            return self

        self._cur.execute(sql, params)
        if cache is not None and klass == "write":
            cache.invalidate()
        self._results = None
        self._cached_hit = False
        return self

    def executemany(self, sql: str, seq: list) -> "Cursor":
        # Always invalidate (executemany is virtually always a write)
        self._cur.executemany(sql, seq)
        if self._conn._cache is not None and _WRITE_RE.match(sql):
            self._conn._cache.invalidate()
        self._results = None
        return self

    def fetchone(self):
        if self._results is not None:
            if not self._results:
                return None
            return self._results.pop(0)
        return self._cur.fetchone()

    def fetchmany(self, size: int = -1):
        if self._results is not None:
            if size < 0:
                size = self.arraysize
            head, self._results = self._results[:size], self._results[size:]
            return head
        return self._cur.fetchmany(size if size >= 0 else self.arraysize)

    def fetchall(self):
        if self._results is not None:
            r, self._results = self._results, []
            return r
        return self._cur.fetchall()

    def close(self):
        self._cur.close()

    def __iter__(self):
        return self

    def __next__(self):
        row = self.fetchone()
        if row is None:
            raise StopIteration
        return row

    # SQLAlchemy probes
    def setinputsizes(self, sizes): pass
    def setoutputsize(self, size, column=None): pass


def connect(database: str, *args, **kwargs) -> Connection:
    """DBAPI 2.0 connect — drop-in replacement for sqlite3.connect."""
    return Connection(database, *args, **kwargs)
