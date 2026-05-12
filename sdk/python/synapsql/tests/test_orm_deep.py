"""Deep ORM tests — FK relationships, transactions, joinedload, bulk-ops."""
import os, tempfile, pytest


def _fresh():
    fd, path = tempfile.mkstemp(suffix=".db")
    os.close(fd)
    return path


def test_orm_foreign_key_relationship():
    pytest.importorskip("sqlalchemy")
    from sqlalchemy import create_engine, Column, Integer, String, ForeignKey
    from sqlalchemy.orm import declarative_base, sessionmaker, relationship

    Base = declarative_base()

    class Company(Base):
        __tablename__ = "companies"
        id = Column(Integer, primary_key=True)
        name = Column(String(100))
        leads = relationship("Lead", back_populates="company")

    class Lead(Base):
        __tablename__ = "leads"
        id = Column(Integer, primary_key=True)
        name = Column(String(100))
        company_id = Column(Integer, ForeignKey("companies.id"))
        company = relationship("Company", back_populates="leads")

    db = _fresh()
    try:
        eng = create_engine(f"synapsql:///{db}")
        Base.metadata.create_all(eng)
        S = sessionmaker(bind=eng)
        s = S()

        c = Company(name="Acme")
        c.leads = [Lead(name="Max"), Lead(name="Anna")]
        s.add(c); s.commit()

        # query w/ relationship
        from sqlalchemy.orm import joinedload
        result = s.query(Company).options(joinedload(Company.leads)).first()
        assert result.name == "Acme"
        assert len(result.leads) == 2
        assert {l.name for l in result.leads} == {"Max", "Anna"}
        s.close(); eng.dispose()
    finally:
        os.unlink(db)


def test_orm_transaction_rollback():
    pytest.importorskip("sqlalchemy")
    from sqlalchemy import create_engine, Column, Integer, String
    from sqlalchemy.orm import declarative_base, sessionmaker

    Base = declarative_base()

    class Item(Base):
        __tablename__ = "items"
        id = Column(Integer, primary_key=True)
        name = Column(String(100))

    db = _fresh()
    try:
        eng = create_engine(f"synapsql:///{db}")
        Base.metadata.create_all(eng)
        S = sessionmaker(bind=eng)
        s = S()
        s.add(Item(name="Keep"))
        s.commit()

        # rollback case
        s.add(Item(name="Discard"))
        s.rollback()

        rows = s.query(Item).all()
        assert len(rows) == 1
        assert rows[0].name == "Keep"
        s.close(); eng.dispose()
    finally:
        os.unlink(db)


def test_orm_bulk_insert_mappings():
    pytest.importorskip("sqlalchemy")
    from sqlalchemy import create_engine, Column, Integer, String
    from sqlalchemy.orm import declarative_base, sessionmaker

    Base = declarative_base()

    class Lead(Base):
        __tablename__ = "leads2"
        id = Column(Integer, primary_key=True)
        name = Column(String(100))
        score = Column(Integer)

    db = _fresh()
    try:
        eng = create_engine(f"synapsql:///{db}")
        Base.metadata.create_all(eng)
        S = sessionmaker(bind=eng)
        s = S()
        # bulk_insert_mappings is fastest ORM-bulk path
        s.bulk_insert_mappings(Lead, [{"name": f"L{i}", "score": i} for i in range(500)])
        s.commit()
        n = s.query(Lead).count()
        assert n == 500
        s.close(); eng.dispose()
    finally:
        os.unlink(db)


def test_orm_query_filter_and_order():
    pytest.importorskip("sqlalchemy")
    from sqlalchemy import create_engine, Column, Integer, String
    from sqlalchemy.orm import declarative_base, sessionmaker

    Base = declarative_base()

    class Lead(Base):
        __tablename__ = "leads3"
        id = Column(Integer, primary_key=True)
        name = Column(String(100))
        score = Column(Integer)

    db = _fresh()
    try:
        eng = create_engine(f"synapsql:///{db}")
        Base.metadata.create_all(eng)
        S = sessionmaker(bind=eng)
        s = S()
        s.add_all([Lead(name=f"L{i}", score=i) for i in range(100)])
        s.commit()

        top = s.query(Lead).filter(Lead.score >= 90).order_by(Lead.score.desc()).limit(5).all()
        assert len(top) == 5
        assert top[0].score == 99
        assert top[-1].score == 95
        s.close(); eng.dispose()
    finally:
        os.unlink(db)


def test_orm_update_invalidates_cache():
    """Critical: ORM UPDATE must trigger cache invalidation for reads."""
    pytest.importorskip("sqlalchemy")
    from sqlalchemy import create_engine, Column, Integer, String
    from sqlalchemy.orm import declarative_base, sessionmaker

    Base = declarative_base()

    class Lead(Base):
        __tablename__ = "leads4"
        id = Column(Integer, primary_key=True)
        status = Column(String(20))

    db = _fresh()
    try:
        eng = create_engine(f"synapsql:///{db}")
        Base.metadata.create_all(eng)
        S = sessionmaker(bind=eng)
        s = S()
        s.add(Lead(id=1, status="new"))
        s.commit()

        # warm cache
        l = s.query(Lead).filter_by(id=1).first()
        assert l.status == "new"

        # update via ORM
        l.status = "won"
        s.commit()
        s.expire_all()  # force ORM reload

        # next read should see new value (cache invalidated)
        l2 = s.query(Lead).filter_by(id=1).first()
        assert l2.status == "won", f"expected 'won', got '{l2.status}' — cache stale?"
        s.close(); eng.dispose()
    finally:
        os.unlink(db)
