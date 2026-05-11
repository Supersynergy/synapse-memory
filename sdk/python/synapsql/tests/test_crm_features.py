"""CRM helpers — tenant-router, audit-log, GDPR, webhooks."""
import os, tempfile, sqlite3, json
import pytest
import synapsql


def test_tenant_router_isolation():
    base = tempfile.mkdtemp()
    try:
        r = synapsql.TenantRouter(base, pool_size_per_tenant=2)

        with r.acquire("acme") as c1:
            c1.execute("CREATE TABLE leads (id INT, name TEXT)")
            c1.execute("INSERT INTO leads VALUES (1, 'Acme-A')")
            c1.commit()

        with r.acquire("contoso") as c2:
            c2.execute("CREATE TABLE leads (id INT, name TEXT)")
            c2.execute("INSERT INTO leads VALUES (1, 'Contoso-X')")
            c2.commit()

        # Verify isolation
        with r.acquire("acme") as c1:
            row = c1.execute("SELECT name FROM leads WHERE id=1").fetchone()
            assert row[0] == "Acme-A"
        with r.acquire("contoso") as c2:
            row = c2.execute("SELECT name FROM leads WHERE id=1").fetchone()
            assert row[0] == "Contoso-X"

        assert set(r.list_tenants()) == {"acme", "contoso"}
        r.close()
    finally:
        import shutil; shutil.rmtree(base)


def test_tenant_router_rejects_traversal():
    base = tempfile.mkdtemp()
    try:
        r = synapsql.TenantRouter(base)
        with pytest.raises(ValueError):
            with r.acquire("../etc/passwd") as _: pass
        with pytest.raises(ValueError):
            with r.acquire("a/b") as _: pass
        r.close()
    finally:
        import shutil; shutil.rmtree(base)


def test_audit_log_chain_signature():
    fd, db = tempfile.mkstemp(suffix=".db")
    os.close(fd)
    try:
        c = sqlite3.connect(db)
        audit = synapsql.AuditLogger(c)
        audit.log("user42", "INSERT", "leads", row_id="1", after={"name": "X"})
        audit.log("user42", "UPDATE", "leads", row_id="1",
                  before={"name": "X"}, after={"name": "Y"})
        audit.log("user42", "DELETE", "leads", row_id="1")
        assert audit.verify_chain() is True

        # Tamper test
        c.execute("UPDATE _audit_log SET after_json = '{\"name\": \"HACKED\"}' WHERE id = 2")
        c.commit()
        assert audit.verify_chain() is False
        c.close()
    finally:
        os.unlink(db)


def test_gdpr_export_anonymize_delete():
    fd, db = tempfile.mkstemp(suffix=".db")
    os.close(fd)
    try:
        c = sqlite3.connect(db)
        c.executescript("""
            CREATE TABLE leads (id INT, email TEXT, name TEXT, status TEXT);
            INSERT INTO leads VALUES (1, 'max@example.de', 'Max Muster', 'won');
            INSERT INTO leads VALUES (2, 'anna@example.de', 'Anna Schmidt', 'new');
        """)
        c.commit()

        audit = synapsql.AuditLogger(c)
        gdpr = synapsql.GdprHelper(c, audit)

        # Export (Art. 15)
        out = gdpr.export_subject("leads", "email", "max@example.de")
        assert out["rows"][0]["name"] == "Max Muster"

        # Anonymize (Art. 17 — pseudonymize)
        n = gdpr.anonymize("leads", "id", 1)
        assert n == 1
        row = c.execute("SELECT name, email, status FROM leads WHERE id = 1").fetchone()
        assert row[0].startswith("REDACTED_")
        assert row[1].startswith("REDACTED_")
        assert row[2] == "won"  # non-PII preserved

        # Hard delete
        n = gdpr.hard_delete("leads", "id", 2)
        assert n == 1
        assert c.execute("SELECT COUNT(*) FROM leads").fetchone()[0] == 1

        # Audit chain intact
        assert audit.verify_chain() is True
        c.close()
    finally:
        os.unlink(db)


def test_webhook_event_dispatch():
    hooks = synapsql.WebhookHooks()
    received = []

    hooks.on("insert:leads", lambda p: received.append(("insert", p)))
    hooks.on("insert:leads", lambda p: received.append(("dup", p)))
    hooks.on("update:deals", lambda p: received.append(("upd", p)))

    hooks.emit("insert:leads", {"id": 1, "name": "X"})
    hooks.emit("update:deals", {"id": 99, "stage": "won"})

    assert len(received) == 3
    assert received[0] == ("insert", {"id": 1, "name": "X"})
    assert received[1] == ("dup", {"id": 1, "name": "X"})
    assert received[2] == ("upd", {"id": 99, "stage": "won"})


def test_webhook_swallows_handler_error():
    hooks = synapsql.WebhookHooks()
    received = []
    hooks.on("e", lambda p: (_ for _ in ()).throw(RuntimeError("boom")))
    hooks.on("e", lambda p: received.append(p))
    hooks.emit("e", {"x": 1})
    assert received == [{"x": 1}]  # second handler still ran
