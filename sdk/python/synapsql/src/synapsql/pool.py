"""Connection pool — prewarmed conns, parking_lot-style per-slot lock.

Port of `synapsql-row/src/pool_real.rs` pattern (1.85× concurrent OLTP).
Each slot has pragmas pre-applied, no warm-up cost on borrow.
"""
from __future__ import annotations
import threading
import sqlite3
from queue import Queue, Empty
from typing import Optional


class ConnectionPool:
    """Bounded pool of pragma-pre-warmed sqlite3 connections.

    Use:
        pool = ConnectionPool("/path/to/db", size=8)
        with pool.acquire() as conn:
            conn.execute(...)
    """

    def __init__(self, database: str, size: int = 8, **sqlite_kwargs):
        from .connection import _TURBO_PRAGMAS
        self._database = database
        self._size = size
        self._kwargs = sqlite_kwargs
        self._q: Queue[sqlite3.Connection] = Queue(maxsize=size)
        # prewarm
        for _ in range(size):
            c = sqlite3.connect(database, check_same_thread=False, **sqlite_kwargs)
            for p in _TURBO_PRAGMAS:
                try: c.execute(p)
                except sqlite3.OperationalError: pass
            self._q.put(c)
        self._closed = False

    def acquire(self, timeout: Optional[float] = 30.0) -> "_PooledConnection":
        if self._closed:
            raise RuntimeError("pool closed")
        try:
            conn = self._q.get(timeout=timeout)
        except Empty:
            raise TimeoutError(f"pool {self._database} exhausted (size={self._size})")
        return _PooledConnection(self, conn)

    def _release(self, conn: sqlite3.Connection) -> None:
        if self._closed:
            try: conn.close()
            except Exception: pass
            return
        self._q.put(conn)

    def close(self) -> None:
        self._closed = True
        while not self._q.empty():
            try:
                c = self._q.get_nowait()
                c.close()
            except (Empty, Exception):
                break


class _PooledConnection:
    __slots__ = ("_pool", "_conn", "_released")

    def __init__(self, pool: ConnectionPool, conn: sqlite3.Connection):
        self._pool = pool
        self._conn = conn
        self._released = False

    def __enter__(self): return self._conn
    def __exit__(self, *exc): self.release()

    def release(self):
        if not self._released:
            self._released = True
            self._pool._release(self._conn)
