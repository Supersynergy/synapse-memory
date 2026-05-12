"""Alembic smoke — verify migrations work via synapsql:// URL."""
import os, tempfile
import pytest


def test_alembic_basic_migration():
    pytest.importorskip("sqlalchemy")
    from sqlalchemy import create_engine, text, MetaData, Table, Column, Integer, String

    fd, db = tempfile.mkstemp(suffix=".db")
    os.close(fd)
    try:
        eng = create_engine(f"synapsql:///{db}")
        meta = MetaData()
        users = Table("users", meta,
                      Column("id", Integer, primary_key=True),
                      Column("name", String(100)),
                      Column("email", String(200)))
        meta.create_all(eng)  # alembic uses same path

        with eng.connect() as conn:
            conn.execute(text("INSERT INTO users(name, email) VALUES('Max', 'm@s.de')"))
            conn.commit()
            r = conn.execute(text("SELECT name FROM users WHERE id=1")).fetchone()
            assert r[0] == "Max"

        # ALTER TABLE (typical alembic op)
        with eng.connect() as conn:
            conn.execute(text("ALTER TABLE users ADD COLUMN phone VARCHAR(20)"))
            conn.execute(text("UPDATE users SET phone='123' WHERE id=1"))
            conn.commit()
            r = conn.execute(text("SELECT phone FROM users WHERE id=1")).fetchone()
            assert r[0] == "123"

        eng.dispose()
    finally:
        os.unlink(db)


def test_orm_session_roundtrip():
    pytest.importorskip("sqlalchemy")
    from sqlalchemy import create_engine, Column, Integer, String
    from sqlalchemy.orm import declarative_base, sessionmaker

    Base = declarative_base()

    class Lead(Base):
        __tablename__ = "leads_orm"
        id = Column(Integer, primary_key=True)
        name = Column(String(100))
        email = Column(String(200))

    fd, db = tempfile.mkstemp(suffix=".db")
    os.close(fd)
    try:
        eng = create_engine(f"synapsql:///{db}")
        Base.metadata.create_all(eng)
        Session = sessionmaker(bind=eng)
        s = Session()
        s.add_all([Lead(name="A", email="a@x"), Lead(name="B", email="b@x")])
        s.commit()
        rows = s.query(Lead).all()
        assert len(rows) == 2
        assert {r.name for r in rows} == {"A", "B"}
        s.close()
        eng.dispose()
    finally:
        os.unlink(db)
