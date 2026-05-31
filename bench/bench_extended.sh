#!/bin/bash
# Synapse vs DuckDB+FTS vs Chroma vs LanceDB vs bare SQLite — 1000 docs.
set -e
cd "$(dirname "$0")/.."

PY=$HOME/.venvs/synbench/bin/python3
N=${N:-1000}
DIR=/tmp/xbench
rm -rf $DIR && mkdir -p $DIR
cd $DIR

$PY -c "
import random, json
random.seed(42)
words='auth token jwt session refresh user admin api cache queue worker shard index vector embedding fts tantivy hnsw sqlite rust python node typescript react nextjs docker deploy bug fix refactor migration schema table column latency bench test'.split()
with open('docs.jsonl','w') as f:
    for i in range($N):
        f.write(json.dumps({'id':i,'title':f'doc{i}','text':' '.join(random.choices(words, k=30))})+'\n')
"

SYNAPSE_BIN=$HOME/projects/synapse/target/release/synapsed
SOCK=/tmp/xbench.sock; SYNDB=$DIR/synapse.db
rm -f $SOCK
$SYNAPSE_BIN -f $SYNDB -s $SOCK --lazy-embed > /tmp/synd_x.log 2>&1 &
PID=$!
sleep 0.4
trap "kill $PID 2>/dev/null; rm -f $SOCK" EXIT

$PY - <<PY
import msgpack, socket, struct, time, json
import duckdb, chromadb, lancedb
import pyarrow as pa

# --- corpus ---
docs = []
with open("docs.jsonl") as f:
    for l in f:
        docs.append(json.loads(l))
N = len(docs)
queries = ["auth","token","bug","fix","cache","shard","admin","react","docker","python"]

R = {}
def bench(name, fn_ins, fn_qry):
    t0=time.perf_counter(); fn_ins(); t1=time.perf_counter()
    ins_ms = (t1-t0)*1000
    t0=time.perf_counter()
    for q in queries: fn_qry(q)
    t1=time.perf_counter()
    qry_ms = (t1-t0)*1000/len(queries)
    R[name] = {"insert_ms": round(ins_ms,2), "lex_ms_per_q": round(qry_ms, 4), "docs_per_sec": round(N/(ins_ms/1000))}
    print(f"  {name}: insert {ins_ms:.1f}ms ({N/(ins_ms/1000):.0f} docs/s), lex {qry_ms:.3f}ms/q")

# --- Synapse ---
print("=== Synapse daemon ===")
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.connect("$SOCK")
def call(r):
    b=msgpack.packb(r); s.sendall(struct.pack("<I",len(b))+b)
    n=struct.unpack("<I", s.recv(4))[0]
    buf=b""
    while len(buf)<n: buf+=s.recv(n-len(buf))
    return msgpack.unpackb(buf, raw=False)
syn_docs=[{"title":d["title"],"uri":None,"text":d["text"],"meta":None,"embed":False} for d in docs]
bench("Synapse",
    lambda: call({"op":"PutBatch","args":syn_docs}),
    lambda q: call({"op":"Search","args":{"mode":"Lex","q":q,"limit":10,"embed_query":False}}))

# --- DuckDB ---
print("=== DuckDB + FTS ===")
c = duckdb.connect("duck.db")
c.execute("INSTALL fts; LOAD fts; CREATE TABLE d(id INTEGER, title VARCHAR, text VARCHAR);")
def d_ins():
    c.executemany("INSERT INTO d VALUES (?,?,?)", [(d["id"],d["title"],d["text"]) for d in docs])
    c.execute("PRAGMA create_fts_index('d','id','title','text');")
bench("DuckDB+FTS",
    d_ins,
    lambda q: c.execute("SELECT id FROM d WHERE fts_main_d.match_bm25(id,?) IS NOT NULL LIMIT 10",(q,)).fetchall())

# --- Chroma (lex: uses metadata filtering, not true FTS) ---
print("=== Chroma (vector-only; lex via keyword-in-text scan) ===")
import chromadb
cc = chromadb.PersistentClient(path="chroma")
col = cc.get_or_create_collection("docs_bench")
# Chroma requires embeddings. Use chroma's default SentenceTransformer embed.
# Measure native path.
def c_ins():
    col.add(
        ids=[str(d["id"]) for d in docs],
        documents=[d["text"] for d in docs],
        metadatas=[{"title": d["title"]} for d in docs],
    )
def c_qry(q):
    col.query(query_texts=[q], n_results=10)
bench("Chroma",
    c_ins,
    c_qry)

# --- LanceDB ---
print("=== LanceDB (FTS + vector) ===")
ldb = lancedb.connect("lance")
# LanceDB requires Arrow table. For FTS benchmark use 'text' scalar column + tantivy index.
arr = pa.Table.from_pylist(
    [{"id": d["id"], "title": d["title"], "text": d["text"]} for d in docs]
)
tbl = ldb.create_table("d", arr, mode="overwrite")
def l_ins():
    # already inserted on create_table. measure index build as "insert cost".
    try:
        tbl.create_fts_index("text", replace=True)
    except Exception as e:
        pass
bench("LanceDB+FTS",
    l_ins,
    lambda q: list(tbl.search(q, query_type="fts").limit(10).to_arrow()))

# --- Bare SQLite FTS5 ---
print("=== Bare SQLite FTS5 (reference floor) ===")
import sqlite3
sc = sqlite3.connect("sqbare.db")
sc.execute("CREATE TABLE d(id INTEGER PRIMARY KEY, title TEXT, text TEXT);")
sc.execute("CREATE VIRTUAL TABLE fts USING fts5(title,text,content='d',content_rowid='id');")
sc.execute("CREATE TRIGGER ai AFTER INSERT ON d BEGIN INSERT INTO fts(rowid,title,text) VALUES(new.id,new.title,new.text); END;")
def s_ins():
    sc.executemany("INSERT INTO d VALUES(?,?,?)", [(d["id"],d["title"],d["text"]) for d in docs])
    sc.commit()
bench("SQLite FTS5",
    s_ins,
    lambda q: list(sc.execute("SELECT rowid FROM fts WHERE fts MATCH ? LIMIT 10",(q,))))

# --- File sizes ---
import os
sizes = {
    "Synapse":   os.path.getsize("$SYNDB"),
    "DuckDB+FTS": os.path.getsize("duck.db"),
    "Chroma":     sum(os.path.getsize(os.path.join(dp,f)) for dp,_,fs in os.walk("chroma") for f in fs),
    "LanceDB+FTS":sum(os.path.getsize(os.path.join(dp,f)) for dp,_,fs in os.walk("lance")  for f in fs),
    "SQLite FTS5":os.path.getsize("sqbare.db"),
}
for k,v in sizes.items(): R[k]["file_bytes"] = v
print("\\n=== SUMMARY ===")
print(f"{'Store':<15} {'insert_ms':>10} {'docs/s':>8} {'lex_ms/q':>10} {'file_bytes':>12}")
for k,v in R.items():
    print(f"{k:<15} {v['insert_ms']:>10.2f} {v['docs_per_sec']:>8} {v['lex_ms_per_q']:>10.4f} {v['file_bytes']:>12}")

with open("$DIR/results.json","w") as f: json.dump(R, f, indent=2)
PY

cp $DIR/results.json ~/projects/synapse/bench/results_extended.json
echo ""
echo "wrote bench/results_extended.json"
