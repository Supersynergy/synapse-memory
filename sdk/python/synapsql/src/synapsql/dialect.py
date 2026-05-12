"""SQLAlchemy dialect — `synapsql://` URLs.

Inherits from SQLite-pysqlite dialect (since brain.db / leadflow.db ARE sqlite),
swaps DBAPI module to our turbo+cache wrapper.
"""
from __future__ import annotations

try:
    from sqlalchemy.dialects.sqlite.pysqlite import SQLiteDialect_pysqlite
except ImportError as e:
    raise ImportError(
        "synapsql.dialect requires SQLAlchemy. Install with: pip install synapsql[sqlalchemy]"
    ) from e


class SynapsqlDialect(SQLiteDialect_pysqlite):
    """`synapsql://` SQLAlchemy dialect — turbo SQLite with hot-cache."""

    name = "synapsql"
    driver = "synapsql"

    # Synapse-tuned brain.db can use cached prepared-stmts safely
    supports_statement_cache = True

    @classmethod
    def import_dbapi(cls):
        from . import connection as dbapi
        return dbapi

    # SQLAlchemy 1.4 fallback
    @classmethod
    def dbapi(cls):
        return cls.import_dbapi()

    def is_disconnect(self, e, connection, cursor):
        # Reuse SQLite's disconnect detection
        return super().is_disconnect(e, connection, cursor)
