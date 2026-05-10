"""Smoke-test DBAPI 2.0 + cache behaviour."""
import os
import tempfile
import pytest

import synapsql


def _fresh_db():
    fd, path = tempfile.mkstemp(suffix=".db")
    os.close(fd)
    return path


def test_connect_and_create():
    db = _fresh_db()
    con = synapsql.connect(db)
    cur = con.cursor()
    cur.execute("CREATE TABLE leads (id INTEGER PRIMARY KEY, company TEXT, score REAL)")
    cur.execute("INSERT INTO leads(company, score) VALUES (?, ?)", ("Acme", 99.5))
    cur.execute("INSERT INTO leads(company, score) VALUES (?, ?)", ("Beta", 50.0))
    con.commit()
    cur.execute("SELECT COUNT(*) FROM leads")
    n = cur.fetchone()[0]
    assert n == 2
    con.close()
    os.unlink(db)


def test_select_cache_hit():
    db = _fresh_db()
    con = synapsql.connect(db)
    con.cursor().execute("CREATE TABLE t (k INTEGER, v TEXT)")
    con.cursor().executemany("INSERT INTO t VALUES (?,?)", [(i, f"row{i}") for i in range(100)])
    con.commit()

    cur1 = con.cursor()
    cur1.execute("SELECT v FROM t WHERE k = ?", (42,))
    r1 = cur1.fetchall()
    assert r1 == [("row42",)]

    # second call → cache hit
    cur2 = con.cursor()
    cur2.execute("SELECT v FROM t WHERE k = ?", (42,))
    assert cur2._cached_hit, "expected cache hit"
    r2 = cur2.fetchall()
    assert r1 == r2

    con.close()
    os.unlink(db)


def test_write_invalidates_cache():
    db = _fresh_db()
    con = synapsql.connect(db)
    con.cursor().execute("CREATE TABLE t (k INTEGER, v TEXT)")
    con.cursor().execute("INSERT INTO t VALUES (1, 'a')")
    con.commit()

    # warm cache
    c = con.cursor()
    c.execute("SELECT v FROM t WHERE k=1")
    assert c.fetchone() == ("a",)
    c2 = con.cursor()
    c2.execute("SELECT v FROM t WHERE k=1")
    assert c2._cached_hit

    # write → invalidate
    con.cursor().execute("UPDATE t SET v='b' WHERE k=1")
    con.commit()

    # next read should miss & return new value
    c3 = con.cursor()
    c3.execute("SELECT v FROM t WHERE k=1")
    assert not c3._cached_hit
    assert c3.fetchone() == ("b",)

    con.close()
    os.unlink(db)


def test_pragmas_applied():
    db = _fresh_db()
    con = synapsql.connect(db)
    cur = con.cursor()
    cur.execute("PRAGMA journal_mode")
    mode = cur.fetchone()[0]
    assert mode.lower() == "wal", f"expected WAL, got {mode}"
    cur.execute("PRAGMA synchronous")
    sync = cur.fetchone()[0]
    assert int(sync) == 1, f"expected NORMAL=1, got {sync}"
    con.close()
    os.unlink(db)


def test_sqlalchemy_dialect():
    pytest.importorskip("sqlalchemy")
    from sqlalchemy import create_engine, text
    db = _fresh_db()
    eng = create_engine(f"synapsql:///{db}")
    with eng.connect() as conn:
        conn.execute(text("CREATE TABLE t (k INT, v TEXT)"))
        conn.execute(text("INSERT INTO t VALUES (1, 'sql')"))
        conn.commit()
        rows = conn.execute(text("SELECT v FROM t")).fetchall()
        assert rows[0][0] == "sql"
    eng.dispose()
    os.unlink(db)
