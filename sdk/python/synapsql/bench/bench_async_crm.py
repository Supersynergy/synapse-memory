"""Async vs sync bench on real SupersynergyCRM leadflow.db.
Tests FastAPI-style concurrent request handling.
"""
import asyncio, time, sqlite3
import synapsql
import synapsql.aio

DB = "/Users/master/projects/SupersynergyCRM/leadflow.db"
N_REQUESTS = 100  # simulate 100 concurrent FastAPI requests

async def async_concurrent_workload():
    conn = await synapsql.aio.connect(DB)
    async def query(i):
        cur = await conn.execute("SELECT id, name, status FROM leads WHERE status='new' ORDER BY rating DESC LIMIT 10")
        return await cur.fetchall()
    t0 = time.perf_counter()
    results = await asyncio.gather(*[query(i) for i in range(N_REQUESTS)])
    elapsed = time.perf_counter() - t0
    await conn.close()
    return elapsed, len(results)

def sync_sequential_workload():
    con = synapsql.connect(DB)
    t0 = time.perf_counter()
    for _ in range(N_REQUESTS):
        cur = con.cursor()
        cur.execute("SELECT id, name, status FROM leads WHERE status='new' ORDER BY rating DESC LIMIT 10")
        cur.fetchall()
    elapsed = time.perf_counter() - t0
    con.close()
    return elapsed

print(f"=== Async vs Sync ({N_REQUESTS} concurrent dashboard queries) ===\n")

async_t, n = asyncio.run(async_concurrent_workload())
sync_t = sync_sequential_workload()

print(f"  async-concurrent: {async_t*1000:.1f}ms total, {async_t/N_REQUESTS*1e6:.1f}µs/req")
print(f"  sync-sequential:  {sync_t*1000:.1f}ms total, {sync_t/N_REQUESTS*1e6:.1f}µs/req")
print(f"\n  Async win: {sync_t/async_t:.2f}× wallclock (FastAPI-like concurrent load)")
