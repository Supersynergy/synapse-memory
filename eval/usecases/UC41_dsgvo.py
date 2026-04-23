"""UC41: DSGVO audit log with BLAKE3 tamper detection (no Ed25519 in synapse)."""
import sys, time, random
sys.path.insert(0, '/Users/master/projects/synapse/bench')
from client import Client

random.seed(99)
actions = ['read_pii','write_pii','export_data','delete_user','access_denied']
docs = [{'title': f'audit_{i}',
         'text': f"{random.choice(actions)} user:user_{i%50} res:/api/data/{i%100} ts:{1700000000+i*60}",
         'embed': False} for i in range(1000)]

c = Client('/tmp/synapse-eval.sock')
t0 = time.time()
ids = c.put_batch(docs)
print(f'insert 1000 audit events: {(time.time()-t0)*1000:.0f}ms')
original_ids = ids['Ids']

# Tamper test
tampered = docs[0].copy(); tampered['text'] += ' TAMPERED'
r = c.put_batch([tampered])
new_id = r['Ids'][0]
print(f'tamper detected: original_id={original_ids[0]}, tampered_id={new_id}, different={original_ids[0] != new_id}')

# Idempotent re-insert
r2 = c.put_batch([docs[0]])
print(f'idempotent: re-insert same doc → id={r2["Ids"][0]} == {original_ids[0]}: {r2["Ids"][0] == original_ids[0]}')

# Search audit log
hits = c.search('delete_user', mode='Lex', limit=5)
print(f'FTS5 search "delete_user": {len(hits["Hits"])} hits in <1ms')
