"""LiveQuery subscribe + flush tests."""
import os, tempfile, sqlite3
import synapsql


def _fresh():
    fd, db = tempfile.mkstemp(suffix=".db"); os.close(fd); return db


def test_live_subscribe_insert():
    db = _fresh()
    try:
        c = sqlite3.connect(db)
        c.execute("CREATE TABLE leads (id INT, name TEXT)")
        c.commit()
        live = synapsql.LiveQuery(c)
        events = []
        live.subscribe("leads", lambda e: events.append(e))

        c.execute("INSERT INTO leads VALUES (1, 'X')")
        c.execute("INSERT INTO leads VALUES (2, 'Y')")
        n = live.commit_and_flush()
        assert n >= 2
        ops = [e["op"] for e in events]
        assert "insert" in ops
        assert all(e["table"] == "leads" for e in events)
        c.close()
    finally:
        os.unlink(db)


def test_live_unsubscribe():
    db = _fresh()
    try:
        c = sqlite3.connect(db)
        c.execute("CREATE TABLE leads (id INT)")
        c.commit()
        live = synapsql.LiveQuery(c)
        received = []
        h = lambda e: received.append(e)
        live.subscribe("leads", h)
        live.unsubscribe("leads", h)
        c.execute("INSERT INTO leads VALUES (1)")
        live.commit_and_flush()
        assert received == []
        c.close()
    finally:
        os.unlink(db)
