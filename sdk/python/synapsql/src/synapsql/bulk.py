"""Bulk-INSERT helper — explicit group-commit with WAL turbo.

Pattern: `synapsql-row/src/batched.rs` — 32× MariaDB single-INSERT @ batch=1000.
Auto-flushes at threshold or on `flush()`. WAL keeps writers concurrent with readers.
"""
from __future__ import annotations
import sqlite3
from typing import Any, Iterable, Optional

from .cache import T0Cache


class BulkWriter:
    """Buffered writer. Append rows, flush on threshold or context-exit.

    Use:
        with BulkWriter(conn, "INSERT INTO leads(name,email) VALUES(?,?)", batch=1000) as w:
            for row in rows:
                w.append(row)
        # auto-flush on exit
    """

    def __init__(
        self,
        conn: sqlite3.Connection,
        sql: str,
        batch: int = 1000,
        cache: Optional[T0Cache] = None,
    ):
        self._conn = conn
        self._sql = sql
        self._batch = batch
        self._buf: list[tuple] = []
        self._cache = cache
        self._total = 0

    def append(self, row: tuple) -> None:
        self._buf.append(row)
        if len(self._buf) >= self._batch:
            self.flush()

    def extend(self, rows: Iterable[tuple]) -> None:
        for row in rows:
            self.append(row)

    def flush(self) -> int:
        if not self._buf:
            return 0
        n = len(self._buf)
        self._conn.executemany(self._sql, self._buf)
        self._buf.clear()
        self._total += n
        if self._cache is not None:
            self._cache.invalidate()
        return n

    @property
    def total(self) -> int:
        return self._total

    def __enter__(self): return self
    def __exit__(self, exc_type, *_):
        if exc_type is None:
            self.flush()
            self._conn.commit()
        else:
            self._conn.rollback()
