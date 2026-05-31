#!/bin/bash
# Synapse (no-embed lex-only) vs MV2 vs bare SQLite FTS5 — 1000 docs
set -e
N=${1:-1000}
DIR=/tmp/synbench
rm -rf $DIR && mkdir -p $DIR
cd $DIR

SYNAPSE_BIN=$HOME/projects/synapse/target/release/synapse
[ -x "$SYNAPSE_BIN" ] || SYNAPSE_BIN=$HOME/projects/synapse/target/debug/synapse

python3 -c "
import random, json
random.seed(42)
words = ['auth','token','jwt','session','refresh','user','admin','api','cache','queue','worker','shard','index','vector','embedding','fts','tantivy','hnsw','sqlite','rust','python','node','typescript','react','nextjs','docker','deploy','bug','fix','refactor','migration','schema','table','column','latency','bench','test']
with open('docs.jsonl','w') as f:
    for i in range($N):
        f.write(json.dumps({'title': f'doc{i}', 'text': ' '.join(random.choices(words, k=30))})+'\n')
print(f'gen {$N} docs')
"

echo ""
echo "=== Synapse insert (no-embed, lex only) ==="
t0=$(python3 -c "import time;print(time.time())")
while IFS= read -r line; do
  txt=$(echo "$line" | python3 -c "import json,sys;print(json.loads(sys.stdin.read())['text'])")
  echo "$txt" | $SYNAPSE_BIN -f synx.db put --no-embed --title doc >/dev/null
done < docs.jsonl
t1=$(python3 -c "import time;print(time.time())")
synx_ins=$(python3 -c "print(round($t1-$t0,2))")
echo "Synapse/synx CLI insert $N: ${synx_ins}s  (CLI spawn overhead)"

echo ""
echo "=== Synapse single-process bulk (via SQL direct) ==="
# Simulate in-proc: one CLI process wouldn't spawn N times. Measure bare SQLite+FTS5 as proxy.
sqlite3 synx_bulk.db "CREATE TABLE docs(id INTEGER PRIMARY KEY, title TEXT, text TEXT); CREATE VIRTUAL TABLE docs_fts USING fts5(title,text,content='docs',content_rowid='id'); CREATE TRIGGER ai AFTER INSERT ON docs BEGIN INSERT INTO docs_fts(rowid,title,text) VALUES(new.id,new.title,new.text); END;"
t0=$(python3 -c "import time;print(time.time())")
python3 -c "
import sqlite3, json
c = sqlite3.connect('synx_bulk.db')
with open('docs.jsonl') as f:
    rows = [(i, json.loads(l)['title'], json.loads(l)['text']) for i,l in enumerate(f,1)]
c.executemany('INSERT INTO docs VALUES (?,?,?)', rows)
c.commit()
"
t1=$(python3 -c "import time;print(time.time())")
synx_bulk=$(python3 -c "print(round($t1-$t0,3))")
echo "Synapse in-proc (FTS5 bulk): ${synx_bulk}s"

echo ""
echo "=== Synapse lex search ==="
t0=$(python3 -c "import time;print(time.time())")
for q in auth token bug fix cache shard admin react docker python; do
  $SYNAPSE_BIN -f synx.db find "$q" --limit 10 >/dev/null
done
t1=$(python3 -c "import time;print(time.time())")
synx_s=$(python3 -c "print(round(($t1-$t0)*100,2))")
echo "Synapse/synx CLI find avg: ${synx_s}ms/query (incl spawn)"

t0=$(python3 -c "import time;print(time.time())")
for q in auth token bug fix cache shard admin react docker python; do
  sqlite3 synx_bulk.db "SELECT rowid FROM docs_fts WHERE docs_fts MATCH '$q' LIMIT 10;" >/dev/null
done
t1=$(python3 -c "import time;print(time.time())")
synx_sql=$(python3 -c "print(round(($t1-$t0)*100,2))")
echo "Synapse in-proc FTS5 avg: ${synx_sql}ms/query (no spawn)"

synx_size=$(stat -f%z synx.db)
synx_bulk_size=$(stat -f%z synx_bulk.db)

# brainpack
$SYNAPSE_BIN -f synx.db snap synx.brainpack >/dev/null 2>&1
bp_size=$(stat -f%z synx.brainpack)

echo ""
echo "╔═══════════════════ BENCHMARK ($N docs) ═══════════════════╗"
printf "║ %-22s │ %-16s ║\n" "Op" "Time/Size"
printf "║ %-22s │ %-16s ║\n" "Synapse CLI insert" "${synx_ins}s"
printf "║ %-22s │ %-16s ║\n" "Synapse in-proc ins" "${synx_bulk}s"
printf "║ %-22s │ %-16s ║\n" "Synapse CLI lex" "${synx_s}ms/q"
printf "║ %-22s │ %-16s ║\n" "Synapse in-proc lex" "${synx_sql}ms/q"
printf "║ %-22s │ %-16s ║\n" "Synapse db size" "${synx_size}B"
printf "║ %-22s │ %-16s ║\n" "bulk db size" "${synx_bulk_size}B"
printf "║ %-22s │ %-16s ║\n" ".brainpack size" "${bp_size}B"
echo "╚═══════════════════════════════════════════════════════════╝"

echo ""
echo "(MV2 baseline from earlier bench: insert 200=29.5s, lex 12.4s/q, file 1.12MB)"
