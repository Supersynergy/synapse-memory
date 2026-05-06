#!/usr/bin/env python3.13
"""Final comprehensive Synapse v1.0.1 benchmark."""
import sys, time
sys.path.insert(0, '/Users/master/.local/lib')
import synxlib
from concurrent.futures import ThreadPoolExecutor

OUT = open("/tmp/bench_final.txt", "w", buffering=1)
def log(s): OUT.write(s+"\n"); OUT.flush(); print(s, flush=True)

def bench(name, fn, runs=3):
    times = []
    for _ in range(runs):
        try:
            t0 = time.time(); fn(); times.append((time.time()-t0)*1000)
        except Exception as e:
            log(f"  {name}: ERROR {str(e)[:80]}"); return
    log(f"  {name:48s} min={min(times):8.3f}ms  med={sorted(times)[len(times)//2]:8.3f}ms")

log(f"=== Synapse v1.0.1 Final Bench {time.strftime('%H:%M:%S')} ===")
log(f"docs={synxlib.stats()['Stats']['docs']}\n")

# === SECTION 1: IPC overhead ===
log("[1] IPC roundtrip overhead")
bench("Ping ×100",      lambda: [synxlib.ping() for _ in range(100)])
bench("Ping ×1000",     lambda: [synxlib.ping() for _ in range(1000)])
bench("Stats ×10",      lambda: [synxlib.stats() for _ in range(10)])

# === SECTION 2: Search ops ===
log("\n[2] Search ops")
queries = ['synapse', 'rabitq', 'python', 'vector', 'test', 'embedding', 'bench', 'rerank', 'ann', 'fts5']
bench("Lex ×10",        lambda: [synxlib.search(q, mode='Lex', limit=10, embed_query=False) for q in queries])
bench("Vec ×10 (server-embed)", lambda: [synxlib.search(q, mode='Vec', limit=10, embed_query=True) for q in queries])
bench("Hybrid ×10 (server-embed)", lambda: [synxlib.search(q, mode='Hybrid', limit=10, embed_query=True) for q in queries])

# === SECTION 3: New BatchSearch ===
log("\n[3] BatchSearch (1 socket roundtrip)")
bench("BatchSearch 10q Lex",      lambda: synxlib.batch_search(queries, mode='Lex'))
bench("BatchSearch 50q Lex",      lambda: synxlib.batch_search(queries*5, mode='Lex'))
bench("BatchSearch 100q Lex",     lambda: synxlib.batch_search(queries*10, mode='Lex'))

# === SECTION 4: Embed cache ===
log("\n[4] Embed cache (client-side LRU)")
import os; os.path.exists(os.path.expanduser("~/.synapse/embed_cache.sqlite")) and os.remove(os.path.expanduser("~/.synapse/embed_cache.sqlite"))
bench("embed_cached miss ×10",    lambda: [synxlib.embed_cached(f"unique_q_{i}_{time.time()}") for i in range(10)])
bench("embed_cached hit ×100",    lambda: [synxlib.embed_cached("synapse vector test") for _ in range(100)])
bench("hybrid_cached ×10",        lambda: [synxlib.hybrid_cached(q, limit=10) for q in queries])

# === SECTION 5: Sql ops ===
log("\n[5] Sql ops via daemon (NEW)")
bench("Sql COUNT(*) docs",        lambda: synxlib.sql("SELECT COUNT(*) FROM docs"))
bench("Sql GROUP BY 177k",        lambda: synxlib.sql("SELECT substr(uri,1,30) p, COUNT(*) c FROM docs GROUP BY p ORDER BY c DESC LIMIT 5"))
bench("Sql FTS5 join",            lambda: synxlib.sql("SELECT d.id FROM docs d WHERE d.id < 100 LIMIT 10"))

# === SECTION 6: Concurrency ===
log("\n[6] Concurrency (8 threads)")
def conc_ping():
    with ThreadPoolExecutor(max_workers=8) as ex:
        list(ex.map(lambda _: synxlib.ping(), range(200)))
bench("8t × 25 pings (200 ops)",  conc_ping)
def conc_lex():
    with ThreadPoolExecutor(max_workers=8) as ex:
        list(ex.map(lambda q: synxlib.search(q, mode='Lex', limit=10, embed_query=False), queries*5))
bench("8t × Lex search (50 ops)", conc_lex)
def conc_sql():
    with ThreadPoolExecutor(max_workers=8) as ex:
        list(ex.map(lambda _: synxlib.sql("SELECT COUNT(*) FROM docs"), range(20)))
bench("8t × Sql COUNT (20 ops)",  conc_sql)

# === SECTION 7: Real-world mix ===
log("\n[7] Real-world workload mix")
def mix():
    synxlib.ping()
    synxlib.stats()
    for q in queries[:5]:
        synxlib.search(q, mode='Lex', limit=10, embed_query=False)
    synxlib.batch_search(queries, mode='Lex')
    synxlib.sql("SELECT COUNT(*) FROM docs")
    for q in queries[:3]:
        synxlib.hybrid_cached(q, limit=10)
bench("mix workload ×3", mix)

# Throughput summary
log("\n=== THROUGHPUT ===")
def measure(label, fn, n_ops):
    t0 = time.time(); fn(); el = time.time()-t0
    log(f"  {label:30s} {n_ops/el:>10.0f} ops/s  ({el*1000:.1f}ms total)")

measure("Ping",      lambda: [synxlib.ping() for _ in range(1000)], 1000)
measure("FTS Lex",   lambda: [synxlib.search(q, mode='Lex', limit=5, embed_query=False) for q in queries*10], 100)
measure("Sql COUNT", lambda: [synxlib.sql("SELECT COUNT(*) FROM docs") for _ in range(100)], 100)
measure("Hybrid cached", lambda: [synxlib.hybrid_cached(q) for q in queries*10], 100)

OUT.close()
print("\nDONE")
