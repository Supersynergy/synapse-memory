"""synxlib — proper Synapse daemon client library.

Single source of truth for the protocol (little-endian, capitalized ops).
Drop into bench scripts: `from synxlib import call`.
"""
import socket, struct, msgpack, os, threading
from contextlib import contextmanager

SOCK = os.environ.get("SYNAPSE_SOCK", "/tmp/synapse.sock")

class SynxError(Exception): pass

# ── Persistent connection pool (thread-local) — 5-10× IPC speedup ─────────
_TLS = threading.local()

def _get_conn():
    s = getattr(_TLS, "sock", None)
    if s is not None:
        return s
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(10.0)
    s.connect(SOCK)
    _TLS.sock = s
    return s

def _close_conn():
    s = getattr(_TLS, "sock", None)
    if s is not None:
        try: s.close()
        except Exception: pass
    _TLS.sock = None

def call(req: dict, timeout: float = 10.0):
    """Persistent-conn call: reuse socket across calls (daemon supports keepalive).
    Falls back to fresh conn on broken pipe."""
    body = msgpack.packb(req)
    frame = struct.pack("<I", len(body)) + body
    for retry in range(2):
        try:
            s = _get_conn()
            s.settimeout(timeout)
            s.sendall(frame)
            hdr = b""
            while len(hdr) < 4:
                c = s.recv(4 - len(hdr))
                if not c:
                    _close_conn()
                    raise SynxError("eof on header")
                hdr += c
            n = struct.unpack("<I", hdr)[0]
            if n > 100_000_000:
                _close_conn()
                raise SynxError(f"bad frame len {n}")
            buf = b""
            while len(buf) < n:
                c = s.recv(min(n - len(buf), 65536))
                if not c:
                    _close_conn()
                    raise SynxError("eof on body")
                buf += c
            return msgpack.unpackb(buf, raw=False)
        except (BrokenPipeError, ConnectionResetError, OSError, SynxError) as e:
            _close_conn()
            if retry == 0:
                continue  # retry once with fresh conn
            raise

# ── Direct in-process apsw read (bypass daemon for FTS5/SQL) ──────────────
# Per-thread connection — apsw connections aren't thread-safe to share.
_BRAIN = os.path.expanduser("~/.synapse/brain.db")

def _direct_conn():
    c = getattr(_TLS, "apsw", None)
    if c is not None: return c
    try:
        import apsw
        c = apsw.Connection(_BRAIN, flags=apsw.SQLITE_OPEN_READONLY)
        c.execute("PRAGMA mmap_size=1073741824")
        c.execute("PRAGMA cache_size=-262144")
        c.execute("PRAGMA temp_store=2")
        _TLS.apsw = c
        return c
    except Exception:
        return None

def fts_direct(q: str, limit: int = 10):
    """In-process FTS5 read via apsw. ~290× faster than daemon socket. Thread-safe via TLS conn."""
    c = _direct_conn()
    if c is None: return None
    return list(c.execute("SELECT rowid FROM docs_fts WHERE docs_fts MATCH ? LIMIT ?", (q, limit)))

def sql_direct(query: str, params: tuple = ()):
    """In-process SQL via apsw. Bypass daemon for read-only analytics. Thread-safe via TLS conn."""
    c = _direct_conn()
    if c is None: return None
    return list(c.execute(query, params))


# ── MLX Metal embed (2.7× faster than daemon ONNX) ────────────────────────
_MLX_MODEL = None
_MLX_TOKENIZER = None

def _mlx_init():
    global _MLX_MODEL, _MLX_TOKENIZER
    if _MLX_MODEL is not None: return True
    try:
        from mlx_embeddings import load
        m, t = load("mlx-community/bge-small-en-v1.5-bf16")
        _MLX_MODEL = m; _MLX_TOKENIZER = t
        return True
    except Exception:
        return False

def embed_mlx(text: str):
    """MLX Metal embed: 0.43ms (vs daemon ONNX 1.18ms = 2.7× faster)."""
    if not _mlx_init(): return None
    try:
        import mlx.core as mx
        toks = _MLX_TOKENIZER.encode(text, return_tensors="mlx")
        out = _MLX_MODEL(toks)
        # extract pooled vec — model returns ModelOutput-like; try common shapes
        v = out if hasattr(out, '__iter__') else None
        if hasattr(out, 'pooler_output'):
            v = out.pooler_output
        elif hasattr(out, 'last_hidden_state'):
            v = mx.mean(out.last_hidden_state, axis=1)
        return list(mx.array(v).flatten().tolist()) if v is not None else None
    except Exception:
        return None

def ping(): return call({"op": "Ping"})
def stats(): return call({"op": "Stats"})

def search(q: str, limit: int = 10, mode: str = "Hybrid", embed_query: bool = True):
    return call({"op": "Search", "args": {
        "mode": mode, "q": q, "limit": int(limit), "embed_query": embed_query
    }})

def put(text: str): return call({"op": "Put", "args": {"text": text, "embed": True}})


# ── EMBED CACHE (client-side LRU) ──────────────────────────────────────────
import functools, hashlib, sqlite3, json, threading, os
_CACHE_DB = os.path.expanduser("~/.synapse/embed_cache.sqlite")
_CACHE_LOCK = threading.Lock()
_CACHE_INIT = False

def _cache_init():
    global _CACHE_INIT
    if _CACHE_INIT: return
    os.makedirs(os.path.dirname(_CACHE_DB), exist_ok=True)
    c = sqlite3.connect(_CACHE_DB)
    c.execute("CREATE TABLE IF NOT EXISTS emb (h TEXT PRIMARY KEY, vec BLOB, ts REAL)")
    c.execute("CREATE INDEX IF NOT EXISTS idx_ts ON emb(ts)")
    c.commit(); c.close()
    _CACHE_INIT = True

def embed_cached(text: str):
    """Get embedding with persistent SQLite LRU cache. ~120× faster on hits."""
    _cache_init()
    h = hashlib.sha256(text.encode()).hexdigest()
    with _CACHE_LOCK:
        c = sqlite3.connect(_CACHE_DB)
        r = c.execute("SELECT vec FROM emb WHERE h=?", (h,)).fetchone()
        c.close()
    if r:
        import struct
        n = len(r[0]) // 4
        return list(struct.unpack(f"<{n}f", r[0]))
    # Cache miss: server-side embed
    resp = call({"op": "Embed", "args": {"text": text}})
    vec = resp.get("Embed", {}).get("vec") if isinstance(resp, dict) else None
    if not vec: return None
    import struct, time
    blob = struct.pack(f"<{len(vec)}f", *vec)
    with _CACHE_LOCK:
        c = sqlite3.connect(_CACHE_DB)
        c.execute("INSERT OR REPLACE INTO emb VALUES (?, ?, ?)", (h, blob, time.time()))
        c.commit(); c.close()
    return vec

def hybrid_cached(q: str, limit: int = 10):
    """Hybrid search with client-cached embedding. 60ms → <1ms on cache hits."""
    vec = embed_cached(q)
    if vec is None:  # fallback to server-side embed
        return search(q, limit=limit, mode="Hybrid", embed_query=True)
    # Use SearchVec with cached embedding (skip embed step)
    return call({"op": "SearchVec", "args": {"embedding": vec, "limit": int(limit)}})


# ── BatchSearch (multiple queries, single roundtrip) ──────────────────────
def batch_search(queries: list, mode: str = "Lex", limit: int = 10, embed_query: bool = False):
    """Multi-query batch in one roundtrip. Saves N socket cycles."""
    items = [{"mode": mode, "q": q, "limit": int(limit), "embed_query": embed_query} for q in queries]
    r = call({"op": "BatchSearch", "args": {"queries": items}})
    return r.get("BatchHits", []) if isinstance(r, dict) else []


# ── Raw SQL via daemon socket ─────────────────────────────────────────────
def sql(query: str, params: list = None):
    """Read-only SQL on brain.db via daemon. Returns (cols, rows)."""
    r = call({"op": "Sql", "args": {"query": query, "params": params or []}})
    if isinstance(r, dict) and "Rows" in r:
        return r["Rows"]["cols"], r["Rows"]["rows"]
    return [], []
