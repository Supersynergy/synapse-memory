"""Tests for v0.6 CRM-v2 helpers."""
import os, tempfile, sqlite3, json
import pytest
import synapsql


def _fresh_db():
    fd, db = tempfile.mkstemp(suffix=".db")
    os.close(fd)
    return db


# 1. SoftDelete
def test_soft_delete_basic():
    db = _fresh_db()
    try:
        c = sqlite3.connect(db)
        c.execute("CREATE TABLE leads (id INT PRIMARY KEY, name TEXT)")
        c.executemany("INSERT INTO leads VALUES (?,?)", [(1,"a"),(2,"b"),(3,"c")])
        c.commit()
        sd = synapsql.SoftDelete(c, "leads", deleted_by_col="deleted_by")
        sd.ensure_columns()
        assert sd.delete(1, by_user_id=42) == 1
        # row still exists but flagged
        n = c.execute("SELECT COUNT(*) FROM leads WHERE is_deleted=1").fetchone()[0]
        assert n == 1
        sd.restore(1)
        assert c.execute("SELECT is_deleted FROM leads WHERE id=1").fetchone()[0] == 0
        c.close()
    finally:
        os.unlink(db)

# 2. TimeTravel
def test_time_travel():
    db = _fresh_db()
    try:
        c = sqlite3.connect(db)
        tt = synapsql.TimeTravel(c)
        tt.snapshot("leads", row_id=1, data={"status": "new", "score": 10})
        tt.snapshot("leads", row_id=1, data={"status": "qualified", "score": 50})
        tt.snapshot("leads", row_id=1, data={"status": "won", "score": 99})
        h = tt.history("leads", 1)
        assert len(h) == 3
        assert h[0]["data"]["status"] == "new"
        assert h[-1]["data"]["status"] == "won"
        # at-time
        latest = tt.at("leads", 1, ts="2099-12-31T00:00:00Z")
        assert latest["status"] == "won"
        c.close()
    finally:
        os.unlink(db)

# 3. FieldEncryption
def test_field_encryption_roundtrip():
    fe = synapsql.FieldEncryption(b"x" * 32)
    plain = "max@example.de"
    ct = fe.encrypt(plain)
    assert ct.startswith("v1:")
    assert plain not in ct
    assert fe.decrypt(ct) == plain

def test_field_encryption_tamper_detect():
    fe = synapsql.FieldEncryption(b"x" * 32)
    ct = fe.encrypt("secret")
    # flip a hex char in ciphertext
    parts = ct.split(":")
    bad_ct = parts[3][:-2] + ("00" if parts[3][-2:] != "00" else "11")
    with pytest.raises(Exception):
        fe.decrypt(":".join(parts[:3] + [bad_ct]))

# 4. SchemaMigrator
def test_schema_migrator_idempotent():
    db = _fresh_db()
    try:
        c = sqlite3.connect(db)
        c.execute("CREATE TABLE leads (id INT)")
        c.commit()
        m = synapsql.SchemaMigrator(c)
        m.add_migration("001_add_score", "ALTER TABLE leads ADD COLUMN score REAL DEFAULT 0")
        m.add_migration("002_index", "CREATE INDEX IF NOT EXISTS ix_score ON leads(score)")
        ran1 = m.run()
        assert set(ran1) == {"001_add_score", "002_index"}
        # second run = no-op
        ran2 = m.run()
        assert ran2 == []
        assert len(m.applied()) == 2
        c.close()
    finally:
        os.unlink(db)

# 5. DataExport
def test_data_export_json_csv():
    db = _fresh_db()
    try:
        c = sqlite3.connect(db)
        c.execute("CREATE TABLE leads (id INT, name TEXT)")
        c.executemany("INSERT INTO leads VALUES (?,?)", [(1,"a"),(2,"b")])
        c.commit()
        ex = synapsql.DataExport(c)
        # JSON inline
        text = ex.to_json("SELECT * FROM leads ORDER BY id")
        rows = json.loads(text)
        assert len(rows) == 2
        # CSV file
        csv_path = tempfile.mkstemp(suffix=".csv")[1]
        n = ex.to_csv("SELECT * FROM leads ORDER BY id", path=csv_path)
        assert n == 2
        assert "name" in open(csv_path).read()
        os.unlink(csv_path)
        c.close()
    finally:
        os.unlink(db)

# 6. Rbac
def test_rbac_grant_check():
    db = _fresh_db()
    try:
        c = sqlite3.connect(db)
        rbac = synapsql.Rbac(c)
        rbac.grant("admin", "*", "*")
        rbac.grant("sales", "leads", "read")
        rbac.grant("sales", "leads", "write")
        rbac.grant("readonly", "leads", "read")

        rbac.assign(user_id=1, role="admin")
        rbac.assign(user_id=2, role="sales")
        rbac.assign(user_id=3, role="readonly")

        assert rbac.can(1, "anything", "delete")  # admin = *,*
        assert rbac.can(2, "leads", "write")
        assert rbac.can(2, "leads", "read")
        assert not rbac.can(3, "leads", "write")
        assert rbac.can(3, "leads", "read")
        c.close()
    finally:
        os.unlink(db)

# 7. BackupRestore
def test_backup_restore():
    db = _fresh_db()
    backup = _fresh_db()
    try:
        c = sqlite3.connect(db)
        c.execute("CREATE TABLE t (k INT, v TEXT)")
        c.execute("INSERT INTO t VALUES (1, 'hello')")
        c.commit(); c.close()

        br = synapsql.BackupRestore(db)
        br.backup(backup)

        c2 = sqlite3.connect(backup)
        v = c2.execute("SELECT v FROM t WHERE k=1").fetchone()[0]
        assert v == "hello"
        c2.close()
    finally:
        os.unlink(db); os.unlink(backup)

# 8. WebhookWorker
def test_webhook_worker_retry_dlq():
    db = _fresh_db()
    try:
        c = sqlite3.connect(db)
        ww = synapsql.WebhookWorker(c, max_attempts=3)
        ww.enqueue("http://example.invalid", {"event": "x"})

        calls = []
        def always_fail(url, body):
            calls.append((url, body))
            raise ConnectionError("simulated")

        # 3 attempts -> DLQ
        for _ in range(3):
            ww.deliver_once(http_post=always_fail)
            # bypass next_attempt_at backoff
            c.execute("UPDATE _webhook_queue SET next_attempt_at=NULL")
            c.commit()

        dlq = ww.dlq()
        assert len(dlq) == 1
        assert "simulated" in (dlq[0][3] or "")
        c.close()
    finally:
        os.unlink(db)

def test_webhook_worker_success():
    db = _fresh_db()
    try:
        c = sqlite3.connect(db)
        ww = synapsql.WebhookWorker(c)
        ww.enqueue("http://example.com", {"event": "x"})
        ww.deliver_once(http_post=lambda url, body: None)  # always succeed
        # delivered
        delivered = c.execute(
            "SELECT COUNT(*) FROM _webhook_queue WHERE delivered_at IS NOT NULL").fetchone()[0]
        assert delivered == 1
        c.close()
    finally:
        os.unlink(db)

# 9. SavedView
def test_saved_view_save_execute():
    db = _fresh_db()
    try:
        c = sqlite3.connect(db)
        c.execute("CREATE TABLE leads (id INT, status TEXT, score REAL)")
        c.executemany("INSERT INTO leads VALUES (?,?,?)",
                      [(1,"new",10),(2,"won",90),(3,"won",95),(4,"lost",5)])
        c.commit()
        sv = synapsql.SavedView(c)
        sv.save(user_id=1, name="hot_won",
                sql="SELECT id FROM leads WHERE status=? AND score>?",
                params=("won", 80))
        rows = sv.execute(1, "hot_won")
        assert sorted(r[0] for r in rows) == [2, 3]
        views = sv.list_for(1)
        assert any(v["name"] == "hot_won" for v in views)
        c.close()
    finally:
        os.unlink(db)

# 10. ActivityFeed
def test_activity_feed():
    db = _fresh_db()
    try:
        c = sqlite3.connect(db)
        af = synapsql.ActivityFeed(c)
        af.record(actor_id=42, verb="created", subject_type="lead", subject_id="1",
                  obj={"name": "Acme"})
        af.record(actor_id=42, verb="updated", subject_type="lead", subject_id="1",
                  obj={"status": "won"})
        af.record(actor_id=99, verb="commented", subject_type="lead", subject_id="1")

        feed = af.for_subject("lead", "1")
        assert len(feed) == 3
        assert feed[0]["verb"] in {"created", "updated", "commented"}

        my = af.for_actor(42)
        assert len(my) == 2
        c.close()
    finally:
        os.unlink(db)
