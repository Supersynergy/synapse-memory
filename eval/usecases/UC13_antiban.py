"""UC13: Per-domain antiban memory from ~/.claude/logs/antiban.db."""
import sys, time, random, sqlite3
sys.path.insert(0, '/Users/master/projects/synapse/bench')
from client import Client

conn = sqlite3.connect('/Users/master/.claude/logs/antiban.db')
hosts = conn.execute('SELECT host, preferred_stage, curl_cffi_ok, curl_cffi_fail, camoufox_needed FROM host_stats').fetchall()
conn.close()

c = Client('/tmp/synapse-eval.sock')
docs = [{'title': h[0], 'text': f'host:{h[0]} stage:{h[1]} ok:{h[2]} fail:{h[3]} camoufox:{h[4]}', 'embed': False} for h in hosts]
t0 = time.time()
c.put_batch(docs)
print(f'insert {len(hosts)} hosts: {(time.time()-t0)*1000:.1f}ms')

lats = []
for h in random.sample(hosts, min(20, len(hosts))):
    tq = time.time()
    c.search(h[0].split('.')[0], mode='Lex', limit=5)
    lats.append((time.time()-tq)*1000)
lats.sort()
print(f'search p50={lats[len(lats)//2]:.2f}ms p95={lats[int(len(lats)*0.95)]:.2f}ms')
