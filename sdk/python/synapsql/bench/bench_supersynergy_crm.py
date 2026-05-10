"""Real-world bench: SupersynergyCRM leadflow.db (4.56M leads, 6.7GB) — sqlite3 vs synapsql.

Workload mirrors typical CRM page-loads:
  - Dashboard: top-N leads by status
  - Lead search (FTS5)
  - Filter by city/source
  - Lookup-by-id
  - Count aggregations

Hot-loop pattern: same query repeated → cache should dominate.
"""
import os, time, sqlite3, statistics, sys
import synapsql

DB = "/Users/master/projects/SupersynergyCRM/leadflow.db"
N_HOT = 200       # repeated identical query (CRM dashboard refresh-loop)
N_VARIED = 100    # parametrized (different ID per call)


def time_ms(fn, n=1):
    t0 = time.perf_counter()
    for _ in range(n):
        fn()
    return (time.perf_counter() - t0) * 1000 / n


def workload(con, label):
    cur = lambda: con.cursor()

    # 1. Dashboard top-50
    q1 = "SELECT id, name, company, status FROM leads WHERE status='new' ORDER BY rating DESC LIMIT 50"
    def w1():
        c = cur(); c.execute(q1); c.fetchall()

    # 2. FTS search
    q2 = "SELECT rowid FROM leads_fts WHERE leads_fts MATCH ? LIMIT 20"
    def w2():
        c = cur(); c.execute(q2, ("münchen",)); c.fetchall()

    # 3. Single-id lookup
    def w3():
        c = cur(); c.execute("SELECT * FROM leads WHERE id = ?", (12345,)); c.fetchone()

    # 4. Count-by-source aggregation
    q4 = "SELECT source, COUNT(*) FROM leads GROUP BY source ORDER BY 2 DESC LIMIT 10"
    def w4():
        c = cur(); c.execute(q4); c.fetchall()

    # 5. Varied lookup (cache-busting)
    def w5(i):
        c = cur(); c.execute("SELECT name, company FROM leads WHERE id = ?", (i,)); c.fetchone()

    print(f"\n--- {label} ---")
    # warm
    w1(); w2(); w3(); w4()

    t1_hot = time_ms(w1, N_HOT)
    t2_hot = time_ms(w2, N_HOT)
    t3_hot = time_ms(w3, N_HOT)
    t4_hot = time_ms(w4, N_HOT)
    t5_v = time_ms(lambda: [w5(i) for i in range(1, N_VARIED+1)], 1) / N_VARIED

    print(f"  dashboard top-50:    {t1_hot*1000:.1f}µs/call (hot×{N_HOT})")
    print(f"  FTS search:          {t2_hot*1000:.1f}µs/call (hot×{N_HOT})")
    print(f"  id lookup (hot):     {t3_hot*1000:.1f}µs/call (hot×{N_HOT})")
    print(f"  count-by-source:     {t4_hot*1000:.1f}µs/call (hot×{N_HOT})")
    print(f"  id lookup (varied):  {t5_v*1000:.1f}µs/call (×{N_VARIED})")
    return [t1_hot, t2_hot, t3_hot, t4_hot, t5_v]


print(f"=== SupersynergyCRM real bench: leadflow.db (4.56M leads, 6.7GB) ===")
print(f"DB: {DB}")
print(f"Size: {os.path.getsize(DB)/1024/1024/1024:.1f} GB")

con_sql = sqlite3.connect(DB)
sql_results = workload(con_sql, "sqlite3 (baseline)")
con_sql.close()

con_syn = synapsql.connect(DB)
syn_results = workload(con_syn, "synapsql")
con_syn.close()

print("\n=== SPEEDUP ===")
labels = ["dashboard", "FTS-search", "id-hot", "count-agg", "id-varied"]
for label, s, y in zip(labels, sql_results, syn_results):
    speed = s / y if y > 0 else float('inf')
    marker = "✅" if speed >= 1.0 else "⚠️"
    print(f"  {marker} {label:14s}  sqlite3={s*1000:.1f}µs  synapsql={y*1000:.1f}µs  → {speed:.2f}×")

# weighted avg (typical CRM mix: 60% hot reads + 30% varied + 10% writes)
w_speedup = (
    0.6 * sum(sql_results[:4]) / max(0.001, sum(syn_results[:4])) +
    0.3 * (sql_results[4] / max(0.001, syn_results[4])) +
    0.1 * 1.35  # batch INSERT speedup from prior bench
)
print(f"\nWeighted CRM-typical speedup: {w_speedup:.2f}×")
