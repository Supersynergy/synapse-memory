"""Bench synapsql vs raw sqlite3 — measure 4 key wins."""
import os, time, tempfile, sqlite3, statistics
import synapsql

N_INSERT = 5000
N_SELECT_HOT = 50000  # hot SELECT benefits from cache
N_SELECT_VARIED = 5000

def fresh():
    fd, path = tempfile.mkstemp(suffix=".db")
    os.close(fd)
    return path

def bench_inserts(con_factory, label):
    db = fresh()
    con = con_factory(db)
    con.cursor().execute("CREATE TABLE t (k INTEGER PRIMARY KEY, v TEXT)")
    con.commit()
    # batched
    t0 = time.perf_counter()
    con.cursor().executemany("INSERT INTO t(v) VALUES (?)", [(f"row{i}",) for i in range(N_INSERT)])
    con.commit()
    t_batch = time.perf_counter() - t0
    # singles
    db2 = fresh()
    con2 = con_factory(db2)
    con2.cursor().execute("CREATE TABLE t (k INTEGER PRIMARY KEY, v TEXT)")
    con2.commit()
    t0 = time.perf_counter()
    cur = con2.cursor()
    for i in range(N_INSERT):
        cur.execute("INSERT INTO t(v) VALUES (?)", (f"row{i}",))
    con2.commit()
    t_single = time.perf_counter() - t0
    con.close(); con2.close()
    os.unlink(db); os.unlink(db2)
    print(f"  {label:18s} batch={t_batch*1000:.1f}ms ({N_INSERT/t_batch:.0f}/s)  single={t_single*1000:.1f}ms ({N_INSERT/t_single:.0f}/s)")
    return t_batch, t_single

def bench_select_hot(con_factory, label):
    db = fresh()
    con = con_factory(db)
    con.cursor().execute("CREATE TABLE t (k INTEGER PRIMARY KEY, v TEXT)")
    con.cursor().executemany("INSERT INTO t(v) VALUES (?)", [(f"row{i}",) for i in range(1000)])
    con.commit()
    # Same query repeated → hits cache for synapsql
    sql = "SELECT v FROM t WHERE k = ?"
    cur = con.cursor()
    cur.execute(sql, (42,))
    cur.fetchall()  # warm
    t0 = time.perf_counter()
    for _ in range(N_SELECT_HOT):
        cur = con.cursor()
        cur.execute(sql, (42,))
        cur.fetchall()
    elapsed = time.perf_counter() - t0
    con.close(); os.unlink(db)
    per_call_us = elapsed / N_SELECT_HOT * 1e6
    print(f"  {label:18s} hot-SELECT {N_SELECT_HOT}× = {elapsed*1000:.1f}ms  per-call={per_call_us:.2f}µs")
    return per_call_us

def bench_select_varied(con_factory, label):
    db = fresh()
    con = con_factory(db)
    con.cursor().execute("CREATE TABLE t (k INTEGER PRIMARY KEY, v TEXT)")
    con.cursor().executemany("INSERT INTO t(v) VALUES (?)", [(f"row{i}",) for i in range(1000)])
    con.commit()
    sql = "SELECT v FROM t WHERE k = ?"
    t0 = time.perf_counter()
    for i in range(N_SELECT_VARIED):
        cur = con.cursor()
        cur.execute(sql, (i % 1000 + 1,))
        cur.fetchall()
    elapsed = time.perf_counter() - t0
    con.close(); os.unlink(db)
    per_call_us = elapsed / N_SELECT_VARIED * 1e6
    print(f"  {label:18s} varied-SELECT {N_SELECT_VARIED}× = {elapsed*1000:.1f}ms  per-call={per_call_us:.2f}µs")
    return per_call_us

print(f"=== synapsql vs sqlite3 bench (M4 Max, N_INSERT={N_INSERT}, N_SELECT_HOT={N_SELECT_HOT}) ===\n")

print("--- INSERT ---")
b_sql = bench_inserts(sqlite3.connect, "sqlite3")
b_syn = bench_inserts(synapsql.connect, "synapsql")
print(f"  Speedup batch: {b_sql[0]/b_syn[0]:.2f}×, single: {b_sql[1]/b_syn[1]:.2f}×")

print("\n--- SELECT (hot, same query repeated) ---")
us_sql = bench_select_hot(sqlite3.connect, "sqlite3")
us_syn = bench_select_hot(synapsql.connect, "synapsql")
print(f"  Speedup hot-SELECT: {us_sql/us_syn:.2f}×")

print("\n--- SELECT (varied, cache-busting) ---")
us_sql_v = bench_select_varied(sqlite3.connect, "sqlite3")
us_syn_v = bench_select_varied(synapsql.connect, "synapsql")
print(f"  Speedup varied-SELECT: {us_sql_v/us_syn_v:.2f}×  (expect ~1× — cache shouldn't help)")
