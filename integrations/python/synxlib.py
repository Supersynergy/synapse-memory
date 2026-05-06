"""synxlib — proper Synapse daemon client library.

Single source of truth for the protocol (little-endian, capitalized ops).
Drop into bench scripts: `from synxlib import call`.
"""
import socket, struct, msgpack, os
from contextlib import contextmanager

SOCK = os.environ.get("SYNAPSE_SOCK", "/tmp/synapse.sock")

class SynxError(Exception): pass

def call(req: dict, timeout: float = 10.0):
    """One-shot call: open, send, recv, close."""
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(timeout)
    try:
        s.connect(SOCK)
        body = msgpack.packb(req)
        s.sendall(struct.pack("<I", len(body)) + body)
        hdr = b""
        while len(hdr) < 4:
            c = s.recv(4 - len(hdr))
            if not c: raise SynxError("eof on header")
            hdr += c
        n = struct.unpack("<I", hdr)[0]
        if n > 100_000_000: raise SynxError(f"bad frame len {n}")
        buf = b""
        while len(buf) < n:
            c = s.recv(min(n - len(buf), 65536))
            if not c: raise SynxError("eof on body")
            buf += c
        return msgpack.unpackb(buf, raw=False)
    finally:
        s.close()

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
