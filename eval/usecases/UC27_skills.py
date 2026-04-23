"""UC27: Skill-index semantic search over ~/.claude/skills.idx."""
import sys, time
sys.path.insert(0, '/Users/master/projects/synapse/bench')
from client import Client

with open('/Users/master/.claude/skills.idx') as f:
    lines = [l.strip() for l in f if l.strip()]

docs = []
for line in lines:
    parts = line.split('\t')
    docs.append({'title': parts[0], 'text': parts[2] if len(parts) > 2 else parts[0], 'embed': True})

c = Client('/tmp/synapse-eval.sock')
print(f'Inserting {len(docs)} skill entries with embeddings (cold-start ~100s)...')
t0 = time.time()
c.put_batch(docs)
print(f'insert: {(time.time()-t0)*1000:.0f}ms')

for q in ['fetch token savings', 'browser automation stealth', 'memory search knowledge']:
    tq = time.time()
    hits = c.search(q, mode='Vec', limit=3, embed_query=True)
    ms = (time.time()-tq)*1000
    print(f'vec q={q!r}: {ms:.1f}ms, top={hits[0]["text"][:60] if hits else "none"}')
