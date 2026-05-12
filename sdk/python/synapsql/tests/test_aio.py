"""Async DBAPI smoke tests."""
import asyncio, os, tempfile
import pytest
import synapsql.aio


def _fresh():
    fd, path = tempfile.mkstemp(suffix=".db")
    os.close(fd)
    return path


def test_async_basic():
    db = _fresh()
    async def go():
        async with await synapsql.aio.connect(db) as conn:
            await conn.execute("CREATE TABLE t (k INT, v TEXT)")
            await conn.execute("INSERT INTO t VALUES (1, 'a')")
            await conn.commit()
            cur = await conn.execute("SELECT v FROM t WHERE k=?", (1,))
            row = await cur.fetchone()
            assert row == ("a",)
    asyncio.run(go())
    os.unlink(db)


def test_async_cache_hit():
    db = _fresh()
    async def go():
        async with await synapsql.aio.connect(db) as conn:
            await conn.execute("CREATE TABLE t (k INT)")
            await conn.executemany("INSERT INTO t VALUES (?)", [(i,) for i in range(50)])
            await conn.commit()
            cur1 = await conn.execute("SELECT k FROM t WHERE k=?", (10,))
            r1 = await cur1.fetchall()
            cur2 = await conn.execute("SELECT k FROM t WHERE k=?", (10,))
            assert cur2._cached_hit
            r2 = await cur2.fetchall()
            assert r1 == r2
    asyncio.run(go())
    os.unlink(db)


def test_async_concurrent():
    db = _fresh()
    async def go():
        async with await synapsql.aio.connect(db) as conn:
            await conn.execute("CREATE TABLE t (k INT, v TEXT)")
            await conn.executemany("INSERT INTO t VALUES (?,?)", [(i, f"r{i}") for i in range(100)])
            await conn.commit()

            async def read(k):
                cur = await conn.execute("SELECT v FROM t WHERE k=?", (k,))
                return await cur.fetchone()

            results = await asyncio.gather(*[read(i) for i in range(20)])
            assert all(r is not None for r in results)
    asyncio.run(go())
    os.unlink(db)
