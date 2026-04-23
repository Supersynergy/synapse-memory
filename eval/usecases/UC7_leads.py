"""UC7: Lead industry shard - 10k synthetic company rows."""
import sys, time, random
sys.path.insert(0, '/Users/master/projects/synapse/bench')
from client import Client

random.seed(7)
industries = ['SaaS','FinTech','E-Commerce','Manufacturing','Consulting','Healthcare','Logistics','Real Estate']
cities = ['München','Berlin','Hamburg','Frankfurt','Stuttgart','Köln','Düsseldorf','Leipzig']

docs = [{'title': f'company_{i}',
         'text': f"{random.choice(industries)} company in {random.choice(cities)} revenue:{random.randint(100,50000)}k employees:{random.randint(1,500)}",
         'embed': False} for i in range(10000)]

c = Client('/tmp/synapse-eval.sock')
t0 = time.time()
c.put_batch(docs)
t1 = time.time()
print(f'insert 10k: {(t1-t0)*1000:.0f}ms ({10000/(t1-t0):.0f}/s)')

lats = []
for q in ['SaaS München','FinTech Berlin','Manufacturing Frankfurt','Healthcare','Logistics Hamburg']:
    tq = time.time()
    c.search(q, mode='Lex', limit=10)
    lats.append((time.time()-tq)*1000)
lats.sort()
print(f'BM25 p50={lats[2]:.2f}ms p95={lats[4]:.2f}ms')
