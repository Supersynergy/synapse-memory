"""AsyncConnectionPool — N prewarmed async-connections for FastAPI.

Each slot wraps a sqlite3.Connection with check_same_thread=False so different
asyncio.to_thread executors can dispatch onto it. Pool size = parallel
event-loop tasks that may simultaneously run DB ops.
"""
from __future__ import annotations
import asyncio
import sqlite3
from collections import deque
from typing import Optional

from .aio import AsyncConnection
from .connection import _TURBO_PRAGMAS


class AsyncConnectionPool:
    """Async-aware pool. acquire() yields an AsyncConnection, release returns it.

    Use:
        pool = await synapsql.aio.AsyncConnectionPool.create(db, size=8)
        async with pool.acquire() as conn:
            cur = await conn.execute("SELECT ...")
            ...
        await pool.close()
    """

    def __init__(self, database: str, size: int):
        self._database = database
        self._size = size
        self._idle: deque[AsyncConnection] = deque()
        self._sem = asyncio.Semaphore(size)
        self._closed = False

    @classmethod
    async def create(cls, database: str, size: int = 8, **sqlite_kwargs) -> "AsyncConnectionPool":
        pool = cls(database, size)
        def _make_conn():
            c = sqlite3.connect(database, check_same_thread=False, **sqlite_kwargs)
            for p in _TURBO_PRAGMAS:
                try: c.execute(p)
                except sqlite3.OperationalError: pass
            return c
        for _ in range(size):
            sc = await asyncio.to_thread(_make_conn)
            pool._idle.append(AsyncConnection(sc, database))
        return pool

    def acquire(self) -> "_AsyncPooledCtx":
        return _AsyncPooledCtx(self)

    async def _borrow(self) -> AsyncConnection:
        if self._closed:
            raise RuntimeError("pool closed")
        await self._sem.acquire()
        if not self._idle:
            # rare race; create on-demand fallback (could be tuned)
            await asyncio.sleep(0)
        return self._idle.popleft()

    def _release(self, conn: AsyncConnection) -> None:
        self._idle.append(conn)
        self._sem.release()

    async def close(self) -> None:
        self._closed = True
        for c in list(self._idle):
            await c.close()
        self._idle.clear()


class _AsyncPooledCtx:
    def __init__(self, pool: AsyncConnectionPool):
        self._pool = pool
        self._conn: Optional[AsyncConnection] = None

    async def __aenter__(self) -> AsyncConnection:
        self._conn = await self._pool._borrow()
        return self._conn

    async def __aexit__(self, *_):
        if self._conn is not None:
            self._pool._release(self._conn)
            self._conn = None
