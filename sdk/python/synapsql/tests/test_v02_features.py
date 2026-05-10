"""Tests v0.2 — pool, bulk, stmt-class cache."""
import os, tempfile, pytest, threading
import synapsql


def _fresh():
    fd, path = tempfile.mkstemp(suffix=".db")
    os.close(fd)
    return path


def test_pool_acquire_release():
    db = _fresh()
    # seed schema
    c = synapsql.connect(db)
    c.cursor().execute("CREATE TABLE t (k INT, v TEXT)")
    c.cursor().execute("INSERT INTO t VALUES (1, 'a')")
    c.commit(); c.close()

    pool = synapsql.ConnectionPool(db, size=4)
    with pool.acquire() as conn:
        cur = conn.execute("SELECT v FROM t WHERE k=1")
        assert cur.fetchone() == ("a",)
    pool.close()
    os.unlink(db)


def test_pool_concurrent():
    db = _fresh()
    c = synapsql.connect(db)
    c.cursor().execute("CREATE TABLE t (k INT, v TEXT)")
    c.cursor().executemany("INSERT INTO t VALUES (?,?)", [(i, f"r{i}") for i in range(100)])
    c.commit(); c.close()

    pool = synapsql.ConnectionPool(db, size=4)
    results = []
    lock = threading.Lock()

    def worker(start):
        with pool.acquire() as conn:
            cur = conn.execute("SELECT v FROM t WHERE k=?", (start,))
            with lock:
                results.append(cur.fetchone()[0])

    threads = [threading.Thread(target=worker, args=(i,)) for i in range(50)]
    for t in threads: t.start()
    for t in threads: t.join()

    assert len(results) == 50
    pool.close()
    os.unlink(db)


def test_bulk_writer_threshold_flush():
    db = _fresh()
    con = synapsql.connect(db)
    con.cursor().execute("CREATE TABLE t (id INT, name TEXT)")
    con.commit()

    sql_con = con._sqlite  # raw connection for BulkWriter
    with synapsql.BulkWriter(sql_con, "INSERT INTO t VALUES (?,?)", batch=100) as w:
        for i in range(250):
            w.append((i, f"name{i}"))
    con.commit()

    cur = con.cursor()
    cur.execute("SELECT COUNT(*) FROM t")
    assert cur.fetchone()[0] == 250
    con.close()
    os.unlink(db)


def test_stmt_class_cache():
    db = _fresh()
    con = synapsql.connect(db)
    con.cursor().execute("CREATE TABLE t (k INT)")
    con.commit()

    cur = con.cursor()
    # first call populates classify-cache
    cur.execute("SELECT k FROM t")
    assert "SELECT k FROM t" in cur._stmt_class
    assert cur._stmt_class["SELECT k FROM t"] == "select"

    cur.execute("INSERT INTO t VALUES (1)")
    assert cur._stmt_class.get("INSERT INTO t VALUES (1)") == "write"
    con.close()
    os.unlink(db)
