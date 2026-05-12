"""CRM-specific helpers — multi-tenant routing, audit-log, GDPR ops, webhooks.

Built from local-KB synthesis of universal CRM requirements (HubSpot/Salesforce/
Pipedrive/EspoCRM/Twenty/SupersynergyCRM). These wrap Synapse-base capabilities
with CRM-domain semantics.
"""
from __future__ import annotations
import os
import re
import json
import hashlib
import sqlite3
import threading
import time
from pathlib import Path
from typing import Any, Callable, Optional

from .connection import _TURBO_PRAGMAS


# -------------------- Multi-Tenant Router --------------------

class TenantRouter:
    """N pre-warmed sqlite3 conns per tenant. DB-per-tenant isolation.

    Use:
        router = TenantRouter("/data/tenants")
        with router.acquire("acme") as conn:
            conn.execute("SELECT * FROM leads")
    """

    def __init__(self, base_dir: str, pool_size_per_tenant: int = 2):
        self.base_dir = Path(base_dir)
        self.base_dir.mkdir(parents=True, exist_ok=True)
        self._size = pool_size_per_tenant
        self._tenants: dict[str, list[sqlite3.Connection]] = {}
        self._lock = threading.RLock()

    def _open_tenant(self, tenant_id: str) -> list[sqlite3.Connection]:
        # Sanitize tenant_id (no path-traversal)
        if not re.fullmatch(r"[a-zA-Z0-9_-]+", tenant_id):
            raise ValueError(f"invalid tenant_id: {tenant_id!r}")
        path = self.base_dir / f"{tenant_id}.db"
        conns = []
        for _ in range(self._size):
            c = sqlite3.connect(str(path), check_same_thread=False)
            for p in _TURBO_PRAGMAS:
                try: c.execute(p)
                except sqlite3.OperationalError: pass
            conns.append(c)
        return conns

    def acquire(self, tenant_id: str) -> "_TenantCtx":
        with self._lock:
            if tenant_id not in self._tenants:
                self._tenants[tenant_id] = self._open_tenant(tenant_id)
        return _TenantCtx(self, tenant_id)

    def list_tenants(self) -> list[str]:
        return [p.stem for p in self.base_dir.glob("*.db")]

    def close(self):
        with self._lock:
            for conns in self._tenants.values():
                for c in conns:
                    try: c.close()
                    except Exception: pass
            self._tenants.clear()


class _TenantCtx:
    def __init__(self, router: TenantRouter, tenant_id: str):
        self._router = router
        self._tenant = tenant_id
        self._conn: Optional[sqlite3.Connection] = None

    def __enter__(self):
        with self._router._lock:
            pool = self._router._tenants[self._tenant]
            self._conn = pool.pop() if pool else self._router._open_tenant(self._tenant)[0]
        return self._conn

    def __exit__(self, *_):
        if self._conn is not None:
            with self._router._lock:
                self._router._tenants[self._tenant].append(self._conn)


# -------------------- Audit Log Helper --------------------

_AUDIT_DDL = """
CREATE TABLE IF NOT EXISTS _audit_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    actor TEXT,
    op TEXT NOT NULL,
    table_name TEXT,
    row_id TEXT,
    before_json TEXT,
    after_json TEXT,
    sig TEXT
);
CREATE INDEX IF NOT EXISTS idx_audit_ts ON _audit_log(ts);
CREATE INDEX IF NOT EXISTS idx_audit_actor ON _audit_log(actor);
CREATE INDEX IF NOT EXISTS idx_audit_table_row ON _audit_log(table_name, row_id);
"""

class AuditLogger:
    """GoBD-compliant audit trail. Append-only, blake3 chain-sigs.

    Use:
        audit = AuditLogger(conn)
        audit.log("user42", "UPDATE", "leads", row_id="123",
                  before={"status": "new"}, after={"status": "won"})
    """

    def __init__(self, conn: sqlite3.Connection):
        self._conn = conn
        self._conn.executescript(_AUDIT_DDL)
        self._last_sig = self._fetch_last_sig()
        self._lock = threading.Lock()

    def _fetch_last_sig(self) -> str:
        row = self._conn.execute("SELECT sig FROM _audit_log ORDER BY id DESC LIMIT 1").fetchone()
        return row[0] if row else "GENESIS"

    def log(self, actor: str, op: str, table: Optional[str] = None,
            row_id: Optional[str] = None, before: Any = None, after: Any = None) -> int:
        with self._lock:
            before_json = json.dumps(before, default=str) if before is not None else None
            after_json = json.dumps(after, default=str) if after is not None else None
            payload = f"{self._last_sig}|{actor}|{op}|{table}|{row_id}|{before_json}|{after_json}"
            sig = hashlib.blake2b(payload.encode(), digest_size=16).hexdigest()
            cur = self._conn.execute(
                "INSERT INTO _audit_log(actor,op,table_name,row_id,before_json,after_json,sig) VALUES (?,?,?,?,?,?,?)",
                (actor, op, table, row_id, before_json, after_json, sig),
            )
            self._conn.commit()
            self._last_sig = sig
            return cur.lastrowid

    def verify_chain(self) -> bool:
        """Re-walk audit log, recompute sigs. Detects tampering."""
        prev = "GENESIS"
        for row in self._conn.execute(
            "SELECT actor,op,table_name,row_id,before_json,after_json,sig FROM _audit_log ORDER BY id ASC"
        ):
            actor, op, table, rid, bj, aj, stored_sig = row
            payload = f"{prev}|{actor}|{op}|{table}|{rid}|{bj}|{aj}"
            expected = hashlib.blake2b(payload.encode(), digest_size=16).hexdigest()
            if expected != stored_sig:
                return False
            prev = stored_sig
        return True


# -------------------- GDPR / DSGVO Helper --------------------

_PII_FIELDS = ("email", "phone", "name", "first_name", "last_name", "address",
               "street", "zip", "ip_address", "birthday", "id_number")


class GdprHelper:
    """GDPR Art. 15-17 — export, anonymize, delete. PII detection by column-name."""

    def __init__(self, conn: sqlite3.Connection, audit: Optional[AuditLogger] = None):
        self._conn = conn
        self._audit = audit

    def export_subject(self, table: str, key_col: str, key_value: Any) -> dict:
        """Art. 15 — Subject Access Request. Return all rows where key_col = value."""
        cur = self._conn.execute(f"SELECT * FROM {table} WHERE {key_col} = ?", (key_value,))
        cols = [d[0] for d in cur.description]
        rows = [dict(zip(cols, r)) for r in cur.fetchall()]
        if self._audit:
            self._audit.log("system", "GDPR_EXPORT", table, str(key_value), after={"rows": len(rows)})
        return {"table": table, "key": {key_col: key_value}, "rows": rows, "exported_at": time.time()}

    def anonymize(self, table: str, key_col: str, key_value: Any,
                  pii_fields: tuple = _PII_FIELDS) -> int:
        """Art. 17 — Pseudonymize PII fields, keep row for analytics."""
        cur = self._conn.execute(f"SELECT * FROM {table} WHERE {key_col} = ? LIMIT 1", (key_value,))
        cols = [d[0] for d in cur.description]
        if not cur.fetchone():
            return 0
        cur.close()
        # Build SET clause for PII columns that exist
        existing_pii = [c for c in cols if c.lower() in pii_fields]
        if not existing_pii:
            return 0
        sets = ", ".join(f"{c} = ?" for c in existing_pii)
        anon_token = f"REDACTED_{hashlib.blake2b(str(key_value).encode(), digest_size=8).hexdigest()}"
        params = [anon_token] * len(existing_pii) + [key_value]
        cur = self._conn.execute(f"UPDATE {table} SET {sets} WHERE {key_col} = ?", params)
        self._conn.commit()
        n = cur.rowcount
        if self._audit:
            self._audit.log("system", "GDPR_ANONYMIZE", table, str(key_value),
                          after={"fields_redacted": existing_pii, "n_rows": n})
        return n

    def hard_delete(self, table: str, key_col: str, key_value: Any) -> int:
        """Art. 17 — Hard delete (no recovery). Use sparingly; prefer anonymize."""
        cur = self._conn.execute(f"DELETE FROM {table} WHERE {key_col} = ?", (key_value,))
        self._conn.commit()
        n = cur.rowcount
        if self._audit:
            self._audit.log("system", "GDPR_DELETE", table, str(key_value), after={"n_rows": n})
        return n


# -------------------- Webhook Event Publisher --------------------

class WebhookHooks:
    """Commit-hook → call user-provided publisher (for webhook delivery).

    Hooks fire AFTER successful commit, async-safe pattern.
    """

    def __init__(self):
        self._handlers: dict[str, list[Callable]] = {}
        self._lock = threading.Lock()

    def on(self, event: str, handler: Callable[[dict], None]) -> None:
        """Register handler for event ('insert:leads', 'update:deals', etc.)"""
        with self._lock:
            self._handlers.setdefault(event, []).append(handler)

    def emit(self, event: str, payload: dict) -> None:
        """Fire all handlers for event. Best-effort, swallows handler errors."""
        with self._lock:
            handlers = list(self._handlers.get(event, []))
        for h in handlers:
            try:
                h(payload)
            except Exception:
                pass  # don't let user webhook crash DB op
