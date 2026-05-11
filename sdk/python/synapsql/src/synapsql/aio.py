"""Async DBAPI for synapsql — FastAPI-friendly, no thread-pool wrapping in user code.

Uses asyncio.to_thread to keep sqlite3 calls off event-loop. Cache is shared
with sync path, so async + sync coexist on same database with consistent state.
Pattern: aiosqlite-compatible API surface but synapsql-tuned (turbo pragmas + AutoloadCache).
"""
from __future__ import annotations
import asyncio
import sqlite3
from typing import Any, Iterable, Optional

from .cache import T0Cache, CacheKey
from .connection import _TURBO_PRAGMAS, _SELECT_RE, _WRITE_RE, _cache_for


class AsyncConnection:
    """Async wrapper. Each method off-loads to default thread-pool.

    Use:
        conn = await synapsql.aio.connect("/app/leadflow.db")
        cur = await conn.execute("SELECT * FROM leads WHERE id=?", (42,))
        rows = await cur.fetchall()
        await conn.close()
    """

    def __init__(self, sqlite_conn: sqlite3.Connection, database: str):
        self._sqlite = sqlite_conn
        self._database = database
        self._cache = _cache_for(database)
        self._stmt_class: dict = {}
        # Single sqlite3.Connection is not concurrent-safe across threads.
        # Serialize async ops via lock; for true concurrency use AsyncConnectionPool.
        self._lock = asyncio.Lock()

    async def execute(self, sql: str, params: tuple = ()) -> "AsyncCursor":
        async with self._lock:
            return await asyncio.to_thread(self._execute_sync, sql, params)

    def _execute_sync(self, sql: str, params: tuple) -> "AsyncCursor":
        klass = self._stmt_class.get(sql)
        if klass is None:
            klass = "select" if _SELECT_RE.match(sql) else ("write" if _WRITE_RE.match(sql) else "other")
            if len(self._stmt_class) < 256:
                self._stmt_class[sql] = klass

        cur = AsyncCursor(self)
        if klass == "select":
            key = CacheKey.of(sql, tuple(params) if params else ())
            hit = self._cache.get(key)
            if hit is not None:
                cur._results = list(hit[0])
                cur._cached_desc = hit[1]
                cur._cached_hit = True
                return cur
            sc = self._sqlite.cursor()
            sc.execute(sql, params)
            rows = sc.fetchall()
            self._cache.put(key, (rows, sc.description))
            cur._results = rows
            cur._cur = sc
            return cur

        sc = self._sqlite.cursor()
        sc.execute(sql, params)
        if klass == "write":
            self._cache.invalidate()
        cur._cur = sc
        return cur

    async def executemany(self, sql: str, seq: Iterable[tuple]) -> None:
        async with self._lock:
            def _go():
                self._sqlite.executemany(sql, list(seq))
                if _WRITE_RE.match(sql):
                    self._cache.invalidate()
            await asyncio.to_thread(_go)

    async def commit(self) -> None:
        async with self._lock:
            await asyncio.to_thread(self._sqlite.commit)

    async def rollback(self) -> None:
        async with self._lock:
            await asyncio.to_thread(self._sqlite.rollback)

    async def close(self) -> None:
        await asyncio.to_thread(self._sqlite.close)

    async def __aenter__(self): return self
    async def __aexit__(self, *_): await self.close()


class AsyncCursor:
    __slots__ = ("_aconn", "_cur", "_results", "_cached_hit", "_cached_desc")

    def __init__(self, aconn: AsyncConnection):
        self._aconn = aconn
        self._cur: Optional[sqlite3.Cursor] = None
        self._results: Optional[list] = None
        self._cached_hit = False
        self._cached_desc = None

    @property
    def description(self):
        if self._cached_hit:
            return self._cached_desc
        return self._cur.description if self._cur else None

    async def fetchone(self):
        if self._results is not None:
            return self._results.pop(0) if self._results else None
        return await asyncio.to_thread(self._cur.fetchone)

    async def fetchall(self):
        if self._results is not None:
            r, self._results = self._results, []
            return r
        return await asyncio.to_thread(self._cur.fetchall)

    async def fetchmany(self, size: int = 1):
        if self._results is not None:
            head, self._results = self._results[:size], self._results[size:]
            return head
        return await asyncio.to_thread(self._cur.fetchmany, size)


async def connect(database: str, **kwargs) -> AsyncConnection:
    """Async connect — applies turbo pragmas off-loop."""
    def _go():
        c = sqlite3.connect(database, check_same_thread=False, **kwargs)
        for p in _TURBO_PRAGMAS:
            try: c.execute(p)
            except sqlite3.OperationalError: pass
        return c
    sc = await asyncio.to_thread(_go)
    return AsyncConnection(sc, database)
