"""Tests for v0.7 intelligence helpers."""
import os, tempfile, sqlite3
import pytest
import synapsql


def _fresh():
    fd, db = tempfile.mkstemp(suffix=".db"); os.close(fd); return db


def test_vector_search_offline_returns_empty():
    """When daemon unreachable, returns [] (caller can fallback)."""
    vs = synapsql.VectorSearch(sock_path="/nonexistent/socket")
    hits = vs.find_similar("test query", limit=5)
    assert hits == []
    assert not vs.is_online()


def test_smart_dedup_exact_email():
    db = _fresh()
    try:
        c = sqlite3.connect(db)
        c.execute("CREATE TABLE leads (id INT PRIMARY KEY, name TEXT, email TEXT, phone TEXT)")
        c.executemany("INSERT INTO leads VALUES (?,?,?,?)", [
            (1, "Max", "max@example.de", "+49100"),
            (2, "Maximilian", "MAX@example.de", "+49200"),  # email-dup
            (3, "Anna", "anna@example.de", "+49100"),       # phone-dup with 1
            (4, "Peter", "peter@example.de", None),
        ])
        c.commit()
        dd = synapsql.SmartDedup(c, "leads")
        pairs_email = dd.find_duplicates(method="exact", cols=["email"])
        # 1 and 2 share email (case-insensitive)
        assert any(set(p[:2]) == {1, 2} for p in pairs_email)
        pairs_phone = dd.find_duplicates(method="exact", cols=["phone"])
        assert any(set(p[:2]) == {1, 3} for p in pairs_phone)
        c.close()
    finally:
        os.unlink(db)


def test_smart_dedup_fuzzy_name():
    db = _fresh()
    try:
        c = sqlite3.connect(db)
        c.execute("CREATE TABLE leads (id INT PRIMARY KEY, name TEXT)")
        c.executemany("INSERT INTO leads VALUES (?,?)", [
            (1, "Acme Corporation Inc"),
            (2, "Acme Corporation"),       # high-sim
            (3, "Beta Industries"),
            (4, "Acme Corp"),               # mid-sim
        ])
        c.commit()
        dd = synapsql.SmartDedup(c, "leads")
        pairs = dd.find_duplicates(method="fuzzy", col="name", threshold=0.5)
        # at least one pair from {1,2,4}
        ids_in_pairs = {p[0] for p in pairs} | {p[1] for p in pairs}
        assert any(i in ids_in_pairs for i in [1, 2, 4])
        c.close()
    finally:
        os.unlink(db)


def test_next_action_markov():
    db = _fresh()
    try:
        c = sqlite3.connect(db)
        af = synapsql.ActivityFeed(c)
        # build typical sales sequence: contact → qualify → propose → close
        for sid in ["1", "2", "3"]:
            af.record(actor_id=1, verb="contacted", subject_type="lead", subject_id=sid)
            af.record(actor_id=1, verb="qualified", subject_type="lead", subject_id=sid)
            af.record(actor_id=1, verb="proposed", subject_type="lead", subject_id=sid)
        # outlier
        af.record(actor_id=1, verb="contacted", subject_type="lead", subject_id="4")
        af.record(actor_id=1, verb="closed", subject_type="lead", subject_id="4")

        na = synapsql.NextAction(c)
        n_pairs = na.train_from_feed(subject_type="lead")
        assert n_pairs > 0

        # for lead 4 last verb was "closed" → no transitions yet
        # for lead 1 last verb is "proposed" → no successors observed
        # for lead 2 if we add another step, but using last-verb-of-a-subject
        # safer: last verb of lead 1 = "proposed" — no transition trained
        # so use lead 1 ts most recent. We trained on contacted→qualified, qualified→proposed.
        # So for a NEW subject with last verb "contacted", predict "qualified".
        af.record(actor_id=1, verb="contacted", subject_type="lead", subject_id="99")
        preds = na.predict("lead", "99", k=3)
        assert len(preds) > 0
        assert preds[0][0] == "qualified"
        c.close()
    finally:
        os.unlink(db)


def test_anomaly_detect_outliers():
    db = _fresh()
    try:
        c = sqlite3.connect(db)
        c.execute("CREATE TABLE leads (id INT PRIMARY KEY, score REAL)")
        # mostly clustered around 50
        rows = [(i, 50 + (i % 10)) for i in range(1, 101)]
        # outliers
        rows += [(101, 1000), (102, -500)]
        c.executemany("INSERT INTO leads VALUES (?,?)", rows)
        c.commit()

        ad = synapsql.AnomalyDetect(c, "leads", "score")
        stats = ad.fit()
        assert stats["n"] == 102
        outliers = ad.find_outliers(z_threshold=3.0)
        out_ids = {o[0] for o in outliers}
        assert 101 in out_ids
        assert 102 in out_ids
        c.close()
    finally:
        os.unlink(db)


def test_nl_query_hot_leads_in_berlin():
    db = _fresh()
    try:
        c = sqlite3.connect(db)
        c.execute("CREATE TABLE leads (id INT PRIMARY KEY, name TEXT, status TEXT, city TEXT, score REAL, created_at TEXT)")
        c.executemany("INSERT INTO leads VALUES (?,?,?,?,?, datetime('now'))", [
            (1, "Acme",    "won",  "Berlin",   95.0),
            (2, "Beta",    "won",  "München",  90.0),
            (3, "Gamma",   "new",  "Berlin",   30.0),
            (4, "Delta",   "lost", "Berlin",   10.0),
        ])
        c.commit()

        nlq = synapsql.NLQuery(c, table="leads")
        sql, params = nlq.parse("hot leads in berlin")
        # must include status=won + city LIKE %berlin%
        assert "status" in sql.lower()
        assert "city" in sql.lower()
        rows = nlq.execute("hot leads in berlin")
        ids = {r[0] for r in rows}
        assert 1 in ids
        assert 3 not in ids and 4 not in ids and 2 not in ids

        # top 2 best — score is column 4 (id, name, status, city, score)
        rows = nlq.execute("top 2 best leads")
        assert len(rows) == 2
        assert rows[0][4] >= rows[1][4]  # ordered by score DESC
        c.close()
    finally:
        os.unlink(db)


def test_nl_query_score_filter():
    db = _fresh()
    try:
        c = sqlite3.connect(db)
        c.execute("CREATE TABLE leads (id INT PRIMARY KEY, score REAL, status TEXT, city TEXT, created_at TEXT)")
        c.executemany("INSERT INTO leads VALUES (?,?,?,?,datetime('now'))", [
            (1, 95.0, "new", "X"), (2, 50.0, "new", "X"), (3, 99.0, "new", "X"),
        ])
        c.commit()
        nlq = synapsql.NLQuery(c, table="leads")
        sql, _ = nlq.parse("leads with score above 80")
        assert "score > ?" in sql or "score > " in sql
        rows = nlq.execute("leads with score above 80")
        ids = {r[0] for r in rows}
        assert ids == {1, 3}
        c.close()
    finally:
        os.unlink(db)
