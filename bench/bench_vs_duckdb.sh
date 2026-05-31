#!/bin/bash
# Synapse daemon vs DuckDB vs bare SQLite FTS5 — same 1000-doc workload.
set -e
cd "$(dirname "$0")/.."

N=${N:-1000}
DIR=/tmp/dbench
rm -rf $DIR && mkdir -p $DIR
cd $DIR

# ---------- corpus ----------
python3 -c "
import random, json
random.seed(42)
words = 'auth token jwt session refresh user admin api cache queue worker shard index vector embedding fts tantivy hnsw sqlite rust python node typescript react nextjs docker deploy bug fix refactor migration schema table column latency bench test'.split()
with open('docs.jsonl','w') as f:
    for i in range($N):
        f.write(json.dumps({'id':i,'title':f'doc{i}','text':' '.join(random.choices(words, k=30))})+'\n')
print(f'gen {$N} docs')
"

SYNAPSE_BIN=$HOME/projects/synapse/target/release/synapsed
SOCK=/tmp/dbench.sock
SYNDB=$DIR/synapse.db
rm -f $SOCK $SYNDB*
$SYNAPSE_BIN -f $SYNDB -s $SOCK --lazy-embed > /tmp/synd_dbench.log 2>&1 &
PID=$!
sleep 0.4
trap "kill $PID 2>/dev/null; rm -f $SOCK" EXIT

# ---------- SYNAPSE ----------
echo "=== Synapse ==="
python3 - <<PY
import msgpack, socket, struct, time, json
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect("$SOCK")
def call(r):
    b=msgpack.packb(r); s.sendall(struct.pack("<I",len(b))+b)
    n=struct.unpack("<I", s.recv(4))[0]
    buf=b""
    while len(buf)<n: buf+=s.recv(n-len(buf))
    return msgpack.unpackb(buf, raw=False)
docs=[{"title":f"d{i}","uri":None,"text":None,"meta":None,"embed":False} for i in range($N)]
with open("docs.jsonl") as f:
    for i,l in enumerate(f):
        docs[i]["text"]=json.loads(l)["text"]
t0=time.perf_counter(); call({"op":"PutBatch","args":docs}); t1=time.perf_counter()
print(f"  insert {$N}: {(t1-t0)*1000:.1f}ms ({$N/(t1-t0):.0f} docs/s)")
qs=["auth","token","bug","fix","cache","shard","admin","react","docker","python"]
t0=time.perf_counter()
for q in qs: call({"op":"Search","args":{"mode":"Lex","q":q,"limit":10,"embed_query":False}})
t1=time.perf_counter()
print(f"  lex avg:   {(t1-t0)*1000/len(qs):.3f}ms/q")
PY

SYN_DB_SIZE=$(stat -f%z $SYNDB)

# ---------- BARE SQLITE ----------
echo "=== Bare SQLite + FTS5 ==="
sqlite3 sqbare.db "CREATE TABLE d(id INTEGER PRIMARY KEY, title TEXT, text TEXT); CREATE VIRTUAL TABLE fts USING fts5(title,text,content='d',content_rowid='id'); CREATE TRIGGER ai AFTER INSERT ON d BEGIN INSERT INTO fts(rowid,title,text) VALUES(new.id,new.title,new.text); END;"
python3 - <<PY
import sqlite3, json, time
c=sqlite3.connect("sqbare.db")
rows=[]
with open("docs.jsonl") as f:
    for l in f:
        d=json.loads(l); rows.append((d["id"],d["title"],d["text"]))
t0=time.perf_counter()
c.executemany("INSERT INTO d VALUES(?,?,?)", rows); c.commit()
t1=time.perf_counter()
print(f"  insert {$N}: {(t1-t0)*1000:.1f}ms ({$N/(t1-t0):.0f} docs/s)")
qs=["auth","token","bug","fix","cache","shard","admin","react","docker","python"]
t0=time.perf_counter()
for q in qs: list(c.execute("SELECT rowid FROM fts WHERE fts MATCH ? LIMIT 10",(q,)))
t1=time.perf_counter()
print(f"  lex avg:   {(t1-t0)*1000/len(qs):.3f}ms/q")
PY
SQ_DB_SIZE=$(stat -f%z sqbare.db)

# ---------- DUCKDB ----------
echo "=== DuckDB + FTS extension ==="
python3 - <<PY
import duckdb, json, time
c = duckdb.connect("duck.db")
c.execute("INSTALL fts; LOAD fts;")
c.execute("CREATE TABLE d(id INTEGER, title VARCHAR, text VARCHAR);")
rows=[]
with open("docs.jsonl") as f:
    for l in f:
        d=json.loads(l); rows.append((d["id"],d["title"],d["text"]))
t0=time.perf_counter()
c.executemany("INSERT INTO d VALUES (?,?,?)", rows)
c.execute("PRAGMA create_fts_index('d','id','title','text');")
t1=time.perf_counter()
print(f"  insert+idx {$N}: {(t1-t0)*1000:.1f}ms ({$N/(t1-t0):.0f} docs/s)")
qs=["auth","token","bug","fix","cache","shard","admin","react","docker","python"]
t0=time.perf_counter()
for q in qs: c.execute("SELECT id FROM d WHERE fts_main_d.match_bm25(id,?) IS NOT NULL LIMIT 10",(q,)).fetchall()
t1=time.perf_counter()
print(f"  lex avg:   {(t1-t0)*1000/len(qs):.3f}ms/q")
PY

DUCK_SIZE=$(stat -f%z duck.db)

# ---------- Summary ----------
echo ""
echo "╔═══════════════ Insert $N docs, lex-only ═══════════════╗"
printf "║ %-22s │ %-14s │ %-14s ║\n" "Store" "insert" "file size"
printf "║ %-22s │ %-14s │ %-14s ║\n" "Synapse daemon" "see above" "${SYN_DB_SIZE}B"
printf "║ %-22s │ %-14s │ %-14s ║\n" "Bare SQLite FTS5" "see above" "${SQ_DB_SIZE}B"
printf "║ %-22s │ %-14s │ %-14s ║\n" "DuckDB + FTS" "see above" "${DUCK_SIZE}B"
echo "╚══════════════════════════════════════════════════════════╝"
