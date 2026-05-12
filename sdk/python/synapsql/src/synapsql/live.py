"""live.py — LiveQuery subscriptions via SQLite update_hook.

Wraps sqlite3.Connection.set_authorizer-based change detection plus a simple
pub/sub broker. Subscribers receive events on INSERT/UPDATE/DELETE.

Use:
    live = LiveQuery(conn)
    live.subscribe("leads", lambda evt: print(evt))
    # any write to leads table → handler fires post-commit
"""
from __future__ import annotations
import sqlite3
import threading
from collections import defaultdict
from typing import Callable

# SQLite authorizer constants
_SQLITE_INSERT = 18
_SQLITE_UPDATE = 23
_SQLITE_DELETE = 9
_SQLITE_OK = 0

_OP_MAP = {_SQLITE_INSERT: "insert", _SQLITE_UPDATE: "update", _SQLITE_DELETE: "delete"}


class LiveQuery:
    """Real-time change notifications. Pub/sub per table.

    Limitation: post-write event (no row-content; only table+op).
    For row-deltas, combine with TimeTravel snapshots.
    """

    def __init__(self, conn: sqlite3.Connection):
        self._c = conn
        self._handlers: dict[str, list[Callable]] = defaultdict(list)
        self._lock = threading.Lock()
        self._pending: list[tuple[str, str]] = []  # (op, table)
        self._install_authorizer()

    def _install_authorizer(self):
        def auth(op_code, arg1, arg2, db_name, source):
            if op_code in _OP_MAP and arg1:
                # Don't fire handlers immediately (mid-transaction); buffer
                self._pending.append((_OP_MAP[op_code], arg1))
            return _SQLITE_OK
        self._c.set_authorizer(auth)

    def subscribe(self, table: str, handler: Callable[[dict], None]) -> None:
        """Handler signature: handler({op, table})"""
        with self._lock:
            self._handlers[table].append(handler)

    def unsubscribe(self, table: str, handler: Callable) -> None:
        with self._lock:
            try:
                self._handlers[table].remove(handler)
            except ValueError:
                pass

    def flush(self) -> int:
        """Call after commit() to dispatch buffered events. Returns count dispatched."""
        with self._lock:
            pending = self._pending
            self._pending = []
        n = 0
        for op, table in pending:
            for h in self._handlers.get(table, []):
                try:
                    h({"op": op, "table": table})
                    n += 1
                except Exception:
                    pass
        return n

    def commit_and_flush(self) -> int:
        """Convenience: commit + flush in one call."""
        self._c.commit()
        return self.flush()
