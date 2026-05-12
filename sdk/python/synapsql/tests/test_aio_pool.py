"""Async pool tests."""
import asyncio, os, tempfile
import synapsql


def _fresh():
    fd, path = tempfile.mkstemp(suffix=".db")
    os.close(fd)
    return path


def test_async_pool_basic():
    db = _fresh()
    async def go():
        # seed
        c = synapsql.connect(db)
        c.cursor().execute("CREATE TABLE t (k INT, v TEXT)")
        c.cursor().executemany("INSERT INTO t VALUES (?,?)", [(i, f"r{i}") for i in range(50)])
        c.commit(); c.close()

        pool = await synapsql.AsyncConnectionPool.create(db, size=4)
        async with pool.acquire() as conn:
            cur = await conn.execute("SELECT v FROM t WHERE k=?", (10,))
            assert (await cur.fetchone()) == ("r10",)
        await pool.close()
    asyncio.run(go())
    os.unlink(db)


def test_async_pool_concurrent_8():
    db = _fresh()
    async def go():
        c = synapsql.connect(db)
        c.cursor().execute("CREATE TABLE t (k INT, v TEXT)")
        c.cursor().executemany("INSERT INTO t VALUES (?,?)", [(i, f"r{i}") for i in range(200)])
        c.commit(); c.close()

        pool = await synapsql.AsyncConnectionPool.create(db, size=4)

        async def read(k):
            async with pool.acquire() as conn:
                cur = await conn.execute("SELECT v FROM t WHERE k=?", (k,))
                return await cur.fetchone()

        # 8 concurrent on 4-slot pool → semaphore queues
        results = await asyncio.gather(*[read(i) for i in range(8)])
        assert all(r is not None for r in results)
        assert len(results) == 8
        await pool.close()
    asyncio.run(go())
    os.unlink(db)
