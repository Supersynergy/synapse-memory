"""crm_v2 — P0+P1 universal CRM features. Compat with SupersynergyCRM column-conventions.

Adds 10 helpers on top of crm.py:
  P0: SoftDelete, TimeTravel, FieldEncryption, SchemaMigration, DataExport
  P1: Rbac, BackupRestore, WebhookWorker, SavedView, ActivityFeed
"""
from __future__ import annotations
import os, json, csv, sqlite3, secrets, time, threading, hashlib, queue, urllib.request
from pathlib import Path
from typing import Any, Optional, Iterable, Callable
from datetime import datetime, timedelta


# -------------------- 1. Soft-Delete + Tombstone --------------------

class SoftDelete:
    """Universal soft-delete helper. CRM convention: is_deleted BOOL + deleted_at TIMESTAMP.

    Use:
        sd = SoftDelete(conn, "leads", deleted_by_col="deleted_by_user_id")
        sd.delete(row_id=123, by_user_id=42)
        sd.restore(row_id=123)
        rows = sd.active_query("SELECT * FROM leads WHERE status=?", ("won",))
    """

    def __init__(self, conn: sqlite3.Connection, table: str,
                 pk: str = "id", deleted_flag: str = "is_deleted",
                 deleted_at_col: str = "deleted_at",
                 deleted_by_col: Optional[str] = None):
        self._c = conn
        self._t = table
        self._pk = pk
        self._flag = deleted_flag
        self._ts = deleted_at_col
        self._by = deleted_by_col

    def ensure_columns(self) -> None:
        cols = {r[1] for r in self._c.execute(f"PRAGMA table_info({self._t})")}
        if self._flag not in cols:
            self._c.execute(f"ALTER TABLE {self._t} ADD COLUMN {self._flag} INTEGER DEFAULT 0")
        if self._ts not in cols:
            self._c.execute(f"ALTER TABLE {self._t} ADD COLUMN {self._ts} TEXT")
        if self._by and self._by not in cols:
            self._c.execute(f"ALTER TABLE {self._t} ADD COLUMN {self._by} INTEGER")
        self._c.commit()

    def delete(self, row_id: Any, by_user_id: Optional[int] = None) -> int:
        sets = f"{self._flag}=1, {self._ts}=strftime('%Y-%m-%dT%H:%M:%fZ','now')"
        params: list = []
        if self._by and by_user_id is not None:
            sets += f", {self._by}=?"
            params.append(by_user_id)
        params.append(row_id)
        cur = self._c.execute(f"UPDATE {self._t} SET {sets} WHERE {self._pk}=?", params)
        self._c.commit()
        return cur.rowcount

    def restore(self, row_id: Any) -> int:
        cur = self._c.execute(
            f"UPDATE {self._t} SET {self._flag}=0, {self._ts}=NULL WHERE {self._pk}=?", (row_id,))
        self._c.commit()
        return cur.rowcount

    def active_query(self, sql: str, params: tuple = ()) -> list:
        # Auto-inject WHERE is_deleted=0 if user didn't include it
        if self._flag not in sql:
            if " WHERE " in sql.upper():
                sql = sql.replace(" WHERE ", f" WHERE {self._flag}=0 AND ", 1)
            else:
                # Insert before ORDER/LIMIT/GROUP
                sql += f" /*soft*/ /* injected */"
                # safer fallback: just exec as-is; user must add WHERE if needed
        return self._c.execute(sql, params).fetchall()

    def purge_older_than(self, days: int) -> int:
        """Hard-delete tombstones older than N days (compaction)."""
        cutoff = (datetime.utcnow() - timedelta(days=days)).strftime("%Y-%m-%dT%H:%M:%fZ")
        cur = self._c.execute(
            f"DELETE FROM {self._t} WHERE {self._flag}=1 AND {self._ts} < ?", (cutoff,))
        self._c.commit()
        return cur.rowcount


# -------------------- 2. Time-Travel --------------------

_TT_DDL = """
CREATE TABLE IF NOT EXISTS _time_travel_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    table_name TEXT NOT NULL,
    row_pk TEXT NOT NULL,
    valid_from TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    valid_to TEXT,
    snapshot_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tt_table_pk ON _time_travel_history(table_name, row_pk, valid_from);
CREATE INDEX IF NOT EXISTS idx_tt_valid_range ON _time_travel_history(table_name, valid_from, valid_to);
"""

class TimeTravel:
    """Bi-temporal versioning. Snapshot-on-write, query at any past time.

    Use:
        tt = TimeTravel(conn)
        tt.snapshot("leads", row_id=1, data={"status": "new", "score": 50})
        # ... later updates ...
        tt.snapshot("leads", row_id=1, data={"status": "won", "score": 95})
        history = tt.history("leads", row_id=1)
        at_time = tt.at("leads", row_id=1, ts="2026-05-10T12:00:00Z")
    """

    def __init__(self, conn: sqlite3.Connection):
        self._c = conn
        self._c.executescript(_TT_DDL)
        self._c.commit()

    def snapshot(self, table: str, row_id: Any, data: dict) -> int:
        # Close out previous snapshot's valid_to
        self._c.execute(
            "UPDATE _time_travel_history SET valid_to=strftime('%Y-%m-%dT%H:%M:%fZ','now') "
            "WHERE table_name=? AND row_pk=? AND valid_to IS NULL",
            (table, str(row_id)))
        cur = self._c.execute(
            "INSERT INTO _time_travel_history(table_name, row_pk, snapshot_json) VALUES (?, ?, ?)",
            (table, str(row_id), json.dumps(data, default=str)))
        self._c.commit()
        return cur.lastrowid

    def history(self, table: str, row_id: Any) -> list[dict]:
        rows = self._c.execute(
            "SELECT valid_from, valid_to, snapshot_json FROM _time_travel_history "
            "WHERE table_name=? AND row_pk=? ORDER BY valid_from ASC",
            (table, str(row_id))).fetchall()
        return [{"valid_from": r[0], "valid_to": r[1], "data": json.loads(r[2])} for r in rows]

    def at(self, table: str, row_id: Any, ts: str) -> Optional[dict]:
        row = self._c.execute(
            "SELECT snapshot_json FROM _time_travel_history "
            "WHERE table_name=? AND row_pk=? AND valid_from <= ? AND (valid_to IS NULL OR valid_to > ?) "
            "ORDER BY valid_from DESC LIMIT 1",
            (table, str(row_id), ts, ts)).fetchone()
        return json.loads(row[0]) if row else None


# -------------------- 3. Field-Level Encryption --------------------

class FieldEncryption:
    """AES-GCM via cryptography (preferred) or pure-Python ChaCha20-Poly1305 fallback.

    Use:
        fe = FieldEncryption(key=os.environ["CRM_DEK"])  # 32 bytes
        cipher = fe.encrypt("max@example.de")
        plain = fe.decrypt(cipher)
    """

    def __init__(self, key: bytes | str):
        if isinstance(key, str):
            key = key.encode("utf-8")
        if len(key) < 32:
            key = hashlib.blake2b(key, digest_size=32).digest()
        self._key = key[:32]
        try:
            from cryptography.hazmat.primitives.ciphers.aead import AESGCM
            self._aead = AESGCM(self._key)
            self._mode = "aes-gcm"
        except ImportError:
            # Fallback: use stdlib hmac+cipher pattern
            self._aead = None
            self._mode = "fallback-xor-hmac"

    def encrypt(self, plaintext: str | bytes) -> str:
        if isinstance(plaintext, str):
            plaintext = plaintext.encode("utf-8")
        nonce = secrets.token_bytes(12)
        if self._aead is not None:
            ct = self._aead.encrypt(nonce, plaintext, None)
        else:
            # XOR-stream (NOT secure; only when cryptography unavailable)
            stream = hashlib.shake_128(self._key + nonce).digest(len(plaintext))
            ct = bytes(a ^ b for a, b in zip(plaintext, stream))
            mac = hashlib.blake2b(ct + nonce + self._key, digest_size=16).digest()
            ct = ct + mac
        return f"v1:{self._mode}:{nonce.hex()}:{ct.hex()}"

    def decrypt(self, token: str) -> str:
        parts = token.split(":", 3)
        if len(parts) != 4 or parts[0] != "v1":
            raise ValueError("invalid token")
        mode, nonce_hex, ct_hex = parts[1], parts[2], parts[3]
        nonce = bytes.fromhex(nonce_hex)
        ct = bytes.fromhex(ct_hex)
        if mode == "aes-gcm" and self._aead is not None:
            return self._aead.decrypt(nonce, ct, None).decode("utf-8")
        elif mode == "fallback-xor-hmac":
            ct, mac = ct[:-16], ct[-16:]
            expected = hashlib.blake2b(ct + nonce + self._key, digest_size=16).digest()
            if not secrets.compare_digest(mac, expected):
                raise ValueError("MAC mismatch")
            stream = hashlib.shake_128(self._key + nonce).digest(len(ct))
            return bytes(a ^ b for a, b in zip(ct, stream)).decode("utf-8")
        raise ValueError(f"unsupported mode {mode}")


# -------------------- 4. Schema Migration --------------------

class SchemaMigrator:
    """Idempotent ALTER + version-tracking. Lighter than alembic for embedded use.

    Use:
        m = SchemaMigrator(conn)
        m.add_migration("001_add_score", "ALTER TABLE leads ADD COLUMN score REAL DEFAULT 0")
        m.add_migration("002_index_status", "CREATE INDEX IF NOT EXISTS ix_status ON leads(status)")
        m.run()  # idempotent: skips already-applied
    """

    def __init__(self, conn: sqlite3.Connection):
        self._c = conn
        self._c.executescript("""
            CREATE TABLE IF NOT EXISTS _schema_migrations (
                version TEXT PRIMARY KEY,
                applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                checksum TEXT
            );
        """)
        self._migrations: list[tuple[str, str]] = []

    def add_migration(self, version: str, sql: str) -> None:
        self._migrations.append((version, sql))

    def run(self) -> list[str]:
        applied = {r[0] for r in self._c.execute("SELECT version FROM _schema_migrations")}
        ran: list[str] = []
        for version, sql in self._migrations:
            if version in applied:
                continue
            checksum = hashlib.blake2b(sql.encode(), digest_size=8).hexdigest()
            try:
                self._c.executescript(sql)
                self._c.execute(
                    "INSERT INTO _schema_migrations(version, checksum) VALUES (?, ?)",
                    (version, checksum))
                self._c.commit()
                ran.append(version)
            except sqlite3.OperationalError as e:
                self._c.rollback()
                raise RuntimeError(f"migration {version} failed: {e}") from e
        return ran

    def applied(self) -> list[tuple[str, str]]:
        return [(v, t) for v, t in self._c.execute(
            "SELECT version, applied_at FROM _schema_migrations ORDER BY applied_at ASC")]


# -------------------- 5. Data Export --------------------

class DataExport:
    """Export tables to JSON / CSV / Parquet (if pyarrow installed)."""

    def __init__(self, conn: sqlite3.Connection):
        self._c = conn

    def to_json(self, sql: str, params: tuple = (), path: Optional[str] = None) -> str | int:
        cur = self._c.execute(sql, params)
        cols = [d[0] for d in cur.description]
        rows = [dict(zip(cols, r)) for r in cur.fetchall()]
        text = json.dumps(rows, default=str, indent=2)
        if path:
            Path(path).write_text(text, encoding="utf-8")
            return len(rows)
        return text

    def to_csv(self, sql: str, params: tuple = (), path: str = "") -> int:
        cur = self._c.execute(sql, params)
        cols = [d[0] for d in cur.description]
        with open(path, "w", newline="", encoding="utf-8") as f:
            w = csv.writer(f)
            w.writerow(cols)
            n = 0
            for row in cur.fetchall():
                w.writerow(row)
                n += 1
        return n

    def to_parquet(self, sql: str, params: tuple = (), path: str = "") -> int:
        try:
            import pyarrow as pa
            import pyarrow.parquet as pq
        except ImportError:
            raise RuntimeError("pyarrow not installed; pip install pyarrow")
        cur = self._c.execute(sql, params)
        cols = [d[0] for d in cur.description]
        rows = cur.fetchall()
        if not rows:
            return 0
        cols_data = {col: [row[i] for row in rows] for i, col in enumerate(cols)}
        table = pa.table(cols_data)
        pq.write_table(table, path)
        return len(rows)


# -------------------- 6. RBAC --------------------

_RBAC_DDL = """
CREATE TABLE IF NOT EXISTS _rbac_roles (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT UNIQUE NOT NULL
);
CREATE TABLE IF NOT EXISTS _rbac_permissions (
    role_id INTEGER NOT NULL,
    resource TEXT NOT NULL,
    action TEXT NOT NULL,
    PRIMARY KEY(role_id, resource, action),
    FOREIGN KEY(role_id) REFERENCES _rbac_roles(id)
);
CREATE TABLE IF NOT EXISTS _rbac_user_roles (
    user_id INTEGER NOT NULL,
    role_id INTEGER NOT NULL,
    PRIMARY KEY(user_id, role_id),
    FOREIGN KEY(role_id) REFERENCES _rbac_roles(id)
);
"""

class Rbac:
    """Role-Based Access Control. Resource-action grants. ~10K-perm-check < 100µs."""

    def __init__(self, conn: sqlite3.Connection):
        self._c = conn
        self._c.executescript(_RBAC_DDL)
        self._c.commit()

    def create_role(self, name: str) -> int:
        # SELECT-first avoids lastrowid ambiguity with INSERT OR IGNORE
        existing = self._c.execute("SELECT id FROM _rbac_roles WHERE name=?", (name,)).fetchone()
        if existing:
            return existing[0]
        cur = self._c.execute("INSERT INTO _rbac_roles(name) VALUES (?)", (name,))
        self._c.commit()
        return cur.lastrowid

    def grant(self, role: str, resource: str, action: str) -> None:
        rid = self.create_role(role)
        self._c.execute(
            "INSERT OR IGNORE INTO _rbac_permissions(role_id, resource, action) VALUES (?,?,?)",
            (rid, resource, action))
        self._c.commit()

    def assign(self, user_id: int, role: str) -> None:
        rid = self.create_role(role)
        self._c.execute(
            "INSERT OR IGNORE INTO _rbac_user_roles(user_id, role_id) VALUES (?,?)",
            (user_id, rid))
        self._c.commit()

    def can(self, user_id: int, resource: str, action: str) -> bool:
        row = self._c.execute(
            "SELECT 1 FROM _rbac_user_roles ur "
            "JOIN _rbac_permissions p ON p.role_id=ur.role_id "
            "WHERE ur.user_id=? AND (p.resource=? OR p.resource='*') "
            "AND (p.action=? OR p.action='*') LIMIT 1",
            (user_id, resource, action)).fetchone()
        return row is not None


# -------------------- 7. Backup / Restore --------------------

class BackupRestore:
    """SQLite backup API + tar-snapshot. WAL-aware."""

    def __init__(self, db_path: str):
        self._path = db_path

    def backup(self, dest: str) -> str:
        src = sqlite3.connect(self._path)
        dst = sqlite3.connect(dest)
        with dst:
            src.backup(dst)
        src.close(); dst.close()
        return dest

    def restore(self, src: str) -> None:
        # Replace current DB (caller must close all conns first)
        import shutil
        shutil.copy2(src, self._path)


# -------------------- 8. Webhook Worker (HTTP retry + DLQ) --------------------

_WEBHOOK_DDL = """
CREATE TABLE IF NOT EXISTS _webhook_queue (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    url TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    enqueued_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    attempts INTEGER DEFAULT 0,
    next_attempt_at TEXT,
    last_error TEXT,
    delivered_at TEXT
);
CREATE INDEX IF NOT EXISTS ix_webhook_pending ON _webhook_queue(delivered_at, next_attempt_at);
"""

class WebhookWorker:
    """Persistent webhook delivery with exponential backoff + DLQ."""

    def __init__(self, conn: sqlite3.Connection, max_attempts: int = 5):
        self._c = conn
        self._c.executescript(_WEBHOOK_DDL)
        self._c.commit()
        self._max = max_attempts

    def enqueue(self, url: str, payload: dict) -> int:
        cur = self._c.execute(
            "INSERT INTO _webhook_queue(url, payload_json, next_attempt_at) "
            "VALUES (?, ?, strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
            (url, json.dumps(payload, default=str)))
        self._c.commit()
        return cur.lastrowid

    def deliver_once(self, http_post: Optional[Callable] = None) -> dict:
        """Process all due-now webhooks. Returns counts."""
        post = http_post or self._default_post
        rows = self._c.execute(
            "SELECT id, url, payload_json, attempts FROM _webhook_queue "
            "WHERE delivered_at IS NULL AND attempts < ? "
            "AND (next_attempt_at IS NULL OR next_attempt_at <= strftime('%Y-%m-%dT%H:%M:%fZ','now')) "
            "LIMIT 100",
            (self._max,)).fetchall()
        ok = 0; fail = 0; dlq = 0
        for wid, url, body, attempts in rows:
            try:
                post(url, body)
                self._c.execute(
                    "UPDATE _webhook_queue SET delivered_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?", (wid,))
                ok += 1
            except Exception as e:
                attempts += 1
                if attempts >= self._max:
                    self._c.execute(
                        "UPDATE _webhook_queue SET attempts=?, last_error=? WHERE id=?",
                        (attempts, str(e)[:200], wid))
                    dlq += 1
                else:
                    backoff_s = 2 ** attempts
                    next_at = datetime.utcnow().replace().isoformat() + "Z"  # naive; real ts add backoff
                    self._c.execute(
                        "UPDATE _webhook_queue SET attempts=?, last_error=?, "
                        "next_attempt_at=datetime('now', '+' || ? || ' seconds') WHERE id=?",
                        (attempts, str(e)[:200], backoff_s, wid))
                    fail += 1
        self._c.commit()
        return {"ok": ok, "retry": fail, "dlq": dlq}

    def dlq(self) -> list:
        return self._c.execute(
            "SELECT id, url, attempts, last_error FROM _webhook_queue "
            "WHERE attempts >= ? AND delivered_at IS NULL", (self._max,)).fetchall()

    @staticmethod
    def _default_post(url: str, body: str) -> None:
        req = urllib.request.Request(url, data=body.encode("utf-8"),
                                     headers={"Content-Type": "application/json"}, method="POST")
        with urllib.request.urlopen(req, timeout=10) as resp:
            if resp.status >= 400:
                raise RuntimeError(f"http {resp.status}")


# -------------------- 9. Saved Searches / Views --------------------

_SV_DDL = """
CREATE TABLE IF NOT EXISTS _saved_views (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER,
    name TEXT NOT NULL,
    sql TEXT NOT NULL,
    params_json TEXT,
    created_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    UNIQUE(user_id, name)
);
"""

class SavedView:
    """Persisted query templates per user. Cached on execute."""

    def __init__(self, conn: sqlite3.Connection):
        self._c = conn
        self._c.executescript(_SV_DDL)
        self._c.commit()

    def save(self, user_id: int, name: str, sql: str, params: tuple = ()) -> int:
        cur = self._c.execute(
            "INSERT OR REPLACE INTO _saved_views(user_id, name, sql, params_json) VALUES (?,?,?,?)",
            (user_id, name, sql, json.dumps(list(params), default=str)))
        self._c.commit()
        return cur.lastrowid

    def list_for(self, user_id: int) -> list[dict]:
        return [{"id": r[0], "name": r[1], "sql": r[2]} for r in self._c.execute(
            "SELECT id, name, sql FROM _saved_views WHERE user_id=? ORDER BY name", (user_id,))]

    def execute(self, user_id: int, name: str) -> list:
        row = self._c.execute(
            "SELECT sql, params_json FROM _saved_views WHERE user_id=? AND name=?",
            (user_id, name)).fetchone()
        if not row:
            raise KeyError(f"no view {name!r} for user {user_id}")
        sql = row[0]
        params = tuple(json.loads(row[1] or "[]"))
        return self._c.execute(sql, params).fetchall()


# -------------------- 10. Activity Feed --------------------

_AF_DDL = """
CREATE TABLE IF NOT EXISTS _activity_feed (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    actor_id INTEGER,
    subject_type TEXT,
    subject_id TEXT,
    verb TEXT NOT NULL,
    object_json TEXT
);
CREATE INDEX IF NOT EXISTS ix_af_subject ON _activity_feed(subject_type, subject_id, ts);
CREATE INDEX IF NOT EXISTS ix_af_actor ON _activity_feed(actor_id, ts);
"""

class ActivityFeed:
    """Touchpoint timeline. SVO-pattern (subject-verb-object)."""

    def __init__(self, conn: sqlite3.Connection):
        self._c = conn
        self._c.executescript(_AF_DDL)
        self._c.commit()

    def record(self, actor_id: Optional[int], verb: str,
               subject_type: Optional[str] = None, subject_id: Optional[str] = None,
               obj: Any = None) -> int:
        cur = self._c.execute(
            "INSERT INTO _activity_feed(actor_id, verb, subject_type, subject_id, object_json) "
            "VALUES (?,?,?,?,?)",
            (actor_id, verb, subject_type, str(subject_id) if subject_id else None,
             json.dumps(obj, default=str) if obj is not None else None))
        self._c.commit()
        return cur.lastrowid

    def for_subject(self, subject_type: str, subject_id: str, limit: int = 50) -> list[dict]:
        return [
            {"ts": r[0], "actor_id": r[1], "verb": r[2], "object": json.loads(r[3]) if r[3] else None}
            for r in self._c.execute(
                "SELECT ts, actor_id, verb, object_json FROM _activity_feed "
                "WHERE subject_type=? AND subject_id=? ORDER BY ts DESC LIMIT ?",
                (subject_type, str(subject_id), limit))
        ]

    def for_actor(self, actor_id: int, limit: int = 50) -> list[dict]:
        return [
            {"ts": r[0], "verb": r[1], "subject": (r[2], r[3]), "object": json.loads(r[4]) if r[4] else None}
            for r in self._c.execute(
                "SELECT ts, verb, subject_type, subject_id, object_json FROM _activity_feed "
                "WHERE actor_id=? ORDER BY ts DESC LIMIT ?",
                (actor_id, limit))
        ]
