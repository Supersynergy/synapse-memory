#!/usr/bin/env bash
# Bench the new Rust-core ndarray fast-path vs the running synapsed daemon.
# Reads ~/.synapse/brain.db read-only through a copied socket, never modifies it.
#
# Usage: bash bench/oltp_bench_turbo_core.sh [N_QUERIES=100]

set -euo pipefail

N=${1:-100}
DB="$HOME/.synapse/brain.db"
SOCK="/tmp/synapse-turbobench.sock"
LOG="/tmp/synapse-turbobench.log"
BIN="/Users/master/projects/synapse/target/release/synapsed"

if [[ ! -x "$BIN" ]]; then
  echo "FATAL: $BIN not built. Run: cargo build --release -p synapsed --features synapse-core/embed,synapse-core/turbo" >&2
  exit 1
fi

if [[ ! -f "$DB" ]]; then
  echo "FATAL: $DB not found" >&2
  exit 1
fi

# Kill any prior bench daemon (NOT the production one on /tmp/synapse.sock).
if [[ -S "$SOCK" ]]; then
  PID=$(lsof -t "$SOCK" 2>/dev/null || true)
  [[ -n "${PID:-}" ]] && kill "$PID" 2>/dev/null || true
  rm -f "$SOCK"
fi

echo "[bench] starting daemon: $BIN -f $DB -s $SOCK"
EMB_CACHE="/tmp/synapse-turbobench.emb-cache"
rm -f "$EMB_CACHE"
SYNAPSE_METRICS_ADDR=127.0.0.1:9097 "$BIN" -f "$DB" -s "$SOCK" --lazy-embed --emb-cache "$EMB_CACHE" >"$LOG" 2>&1 &
DAEMON_PID=$!
trap 'kill $DAEMON_PID 2>/dev/null || true; rm -f $SOCK' EXIT

# Wait for socket
for i in {1..50}; do
  [[ -S "$SOCK" ]] && break
  sleep 0.1
done
[[ -S "$SOCK" ]] || { echo "FATAL: socket never appeared. log:"; cat "$LOG"; exit 1; }
sleep 0.5

# Sample query terms (English, drawn from common text)
QUERIES=(
  "rust async runtime"     "machine learning model"  "vector database"
  "graph algorithm"        "compiler optimization"   "memory allocator"
  "distributed system"     "cryptographic hash"      "neural network training"
  "garbage collection"     "concurrent data structure" "B-tree index"
  "operating system kernel" "network protocol"       "file system journaling"
  "JIT compilation"        "type inference"          "lock-free queue"
  "embedding model"        "transformer attention"
)

# Warm up the daemon (this triggers the lazy ndarray build on first vec call)
echo "[bench] warmup (triggers ndarray cache build, may take ~250ms)..."
SYNAPSE_SOCK="$SOCK" syn vec "warmup query" >/dev/null 2>&1 || true
SYNAPSE_SOCK="$SOCK" syn vec "warmup query" >/dev/null 2>&1 || true

echo "[bench] running $N vec queries (in-process timing via python socket client)..."
QSTR=$(printf '"%s",' "${QUERIES[@]}")
SYNAPSE_SOCK="$SOCK" python3 - <<PY
import os, socket, struct, time, statistics, sys
try:
    import msgpack
except ImportError:
    print("FATAL: pip install msgpack required for in-process bench", file=sys.stderr); sys.exit(2)
sock = os.environ["SYNAPSE_SOCK"]
queries = [$QSTR]
def call(req):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.connect(sock)
    body = msgpack.packb(req, use_bin_type=True)
    s.sendall(struct.pack(">I", len(body)) + body)
    hdr = b""
    while len(hdr) < 4: hdr += s.recv(4-len(hdr))
    n = struct.unpack(">I", hdr)[0]
    buf = b""
    while len(buf) < n: buf += s.recv(min(65536, n-len(buf)))
    s.close()
    return msgpack.unpackb(buf, raw=False)
times = []
for i in range($N):
    q = queries[i % len(queries)]
    t0 = time.perf_counter_ns()
    call({"op":"Search","args":{"mode":"Vec","q":q,"limit":10,"embed_query":True}})
    t1 = time.perf_counter_ns()
    times.append((t1-t0)/1e6)
times.sort()
n = len(times)
print(f"  n      = {n}")
print(f"  min    = {times[0]:.2f} ms")
print(f"  p50    = {statistics.median(times):.2f} ms")
print(f"  p95    = {times[int(n*0.95)]:.2f} ms")
print(f"  p99    = {times[int(n*0.99)]:.2f} ms")
print(f"  max    = {times[-1]:.2f} ms")
print(f"  mean   = {statistics.mean(times):.2f} ms")
PY

echo "[bench] daemon RSS:"
ps -o rss= -p "$DAEMON_PID" | awk '{printf "  %.1f MB\n", $1/1024}'

echo "[bench] daemon log tail:"
tail -5 "$LOG" | sed 's/^/  /'

rm -f "$TIMES_FILE"
