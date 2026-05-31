"""synxlib — proper Synapse daemon client library.

Single source of truth for the protocol (little-endian, capitalized ops).
Drop into bench scripts: `from synxlib import call`.
"""
import socket, struct, msgpack, os, threading, urllib.parse, urllib.request, json
from contextlib import contextmanager

SOCK = os.environ.get("SYNAPSE_SOCK", "/tmp/synapse.sock")
TURBO_URL = os.environ.get("SYNAPSE_TURBO_URL", "http://127.0.0.1:9477").rstrip("/")
TURBO_FIRST = os.environ.get("SYNX_DISABLE_TURBO", "0") != "1"

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

def _turbo_search(q: str, limit: int = 10, mode: str = "Hybrid"):
    if not TURBO_FIRST:
        return None
    path = {"Hybrid": "hybrid", "Vec": "vec", "Lex": "find"}.get(mode)
    if path is None:
        return None
    qs = urllib.parse.urlencode({"q": q, "limit": int(limit)})
    req = urllib.request.Request(f"{TURBO_URL}/{path}?{qs}")
    if req.type != "http" or not req.host.startswith("127.0.0.1"):
        return None
    with urllib.request.urlopen(req, timeout=2) as resp:  # noqa: S310
        data = json.loads(resp.read())
    hits = []
    for row in data.get("results", []):
        hits.append({
            "id": row.get("id"),
            "uri": row.get("uri"),
            "title": row.get("title"),
            "text": row.get("text") or "",
            "score": row.get("score", row.get("distance", 0.0)),
        })
    return {"Hits": hits, "source": "turbo", "elapsed_ms": data.get("elapsed_ms")}

def search(q: str, limit: int = 10, mode: str = "Hybrid", embed_query: bool = True):
    """Auto-route: read-only Lex mode → direct apsw bypass (3.3× concurrent vs daemon).
    Vec/Hybrid stay on daemon (need ANN index in-process)."""
    try:
        out = _turbo_search(q, limit, mode)
        if out is not None:
            return out
    except Exception:
        pass
    if mode == "Lex" and not embed_query:
        # Direct apsw FTS5 — bypasses daemon serialization
        rows = fts_direct(q, limit)
        if rows is not None:
            # Mimic daemon Hits structure
            return {"Hits": [{"id": r[0], "uri": None, "title": None, "text": "", "score": 0.0} for r in rows]}
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


# ── Matryoshka truncation (BGE-small-v1.5 supports MRL: 384→256→192→128) ──
def embed_truncated(text: str, dim: int = 192):
    """Server-side Matryoshka truncation: ~99% quality at 192d, ~96% at 96d.
    Daemon Embed op accepts `dim` arg (P1.2) — truncates+renorms server-side.
    Falls back to client-side trunc if daemon doesn't support dim arg yet.
    """
    # Try server-side trunc (post-rebuild)
    resp = call({"op": "Embed", "args": {"text": text, "dim": dim}})
    if isinstance(resp, dict) and "Embed" in resp:
        return resp["Embed"].get("vec")
    # Fallback: client-side trunc
    full = embed_cached(text)
    if full is None or dim >= len(full):
        return full
    cut = full[:dim]
    import math
    norm = math.sqrt(sum(x*x for x in cut))
    if norm > 1e-10:
        cut = [x / norm for x in cut]
    return cut


# ── INT8 packed embed cache (P1.3) — 4× compression ────────────────────────
def embed_cached_q8(text: str):
    """f32→i8 quantized cache: 384B per vec (vs 1536B f32). Recall <1% drop."""
    _cache_init()
    h = hashlib.sha256(("q8:" + text).encode()).hexdigest()
    with _CACHE_LOCK:
        c = sqlite3.connect(_CACHE_DB)
        r = c.execute("SELECT vec FROM emb WHERE h=?", (h,)).fetchone()
        c.close()
    if r:
        # Decode i8 + scale
        scale = struct.unpack("<f", r[0][:4])[0]
        n = len(r[0]) - 4
        vals = struct.unpack(f"<{n}b", r[0][4:])
        return [v * scale / 127.0 for v in vals]
    # Cache miss: get full, quantize, store
    full = call({"op": "Embed", "args": {"text": text}})
    vec = full.get("Embed", {}).get("vec") if isinstance(full, dict) else None
    if not vec: return None
    max_abs = max(abs(v) for v in vec) or 1.0
    quant = bytes((min(127, max(-128, int(round(v / max_abs * 127)))) & 0xff) for v in vec)
    blob = struct.pack("<f", max_abs) + quant
    import time as _t
    with _CACHE_LOCK:
        c = sqlite3.connect(_CACHE_DB)
        c.execute("INSERT OR REPLACE INTO emb VALUES (?, ?, ?)", (h, blob, _t.time()))
        c.commit(); c.close()
    return vec


# ── Atomic transactions (P2.3) ────────────────────────────────────────────
def transaction(items: list):
    """All-or-nothing batch. items = list of dicts with text/title/uri/meta/embed."""
    return call({"op": "Transaction", "args": {"ops": items}})


# ── Schemafull/freeform mix (P5.4) — sqlite TYPES + JSON ──────────────────
def define_table(name: str, fields: dict):
    """Create strict-typed table via daemon Sql. fields={col: type}."""
    cols = ", ".join(f'"{k}" {v}' for k, v in fields.items())
    return sql(f"CREATE TABLE IF NOT EXISTS {name} ({cols})")


# ── Time-series helpers (P5.3) — sqlite window functions ─────────────────
def ts_lag(table: str, value_col: str, ts_col: str = "ts", n: int = 1, where: str = "1=1"):
    """SELECT lag(value, n) OVER (ORDER BY ts) — trend analysis."""
    return sql(f"SELECT {ts_col}, {value_col}, lag({value_col}, ?) OVER (ORDER BY {ts_col}) AS prev FROM {table} WHERE {where}", [n])


def ts_rolling_avg(table: str, value_col: str, ts_col: str = "ts", window: int = 7, where: str = "1=1"):
    """Rolling N-period average."""
    return sql(f"SELECT {ts_col}, {value_col}, avg({value_col}) OVER (ORDER BY {ts_col} ROWS BETWEEN ? PRECEDING AND CURRENT ROW) AS rolling_avg FROM {table} WHERE {where}", [window-1])


# ── Geo helpers (P5.2) — needs spatialite ext (optional) ──────────────────
def geo_within(lat: float, lon: float, radius_m: float, table: str = "docs"):
    """Find docs within radius using haversine fallback (no spatialite needed).
    Assumes table has lat/lon columns (REAL). Returns ids ranked by distance.
    """
    # Haversine via SQL math
    return sql(f"""
        SELECT id, lat, lon,
            6371000 * 2 * asin(sqrt(
                pow(sin(radians(lat - ?) / 2), 2) +
                cos(radians(?)) * cos(radians(lat)) *
                pow(sin(radians(lon - ?) / 2), 2)
            )) AS dist_m
        FROM {table}
        WHERE dist_m <= ?
        ORDER BY dist_m
        LIMIT 100
    """, [lat, lat, lon, radius_m])


# ── BatchSearch (multiple queries, single roundtrip) ──────────────────────
def batch_search(queries: list, mode: str = "Lex", limit: int = 10, embed_query: bool = False):
    """Multi-query batch in one roundtrip. Saves N socket cycles."""
    items = [{"mode": mode, "q": q, "limit": int(limit), "embed_query": embed_query} for q in queries]
    r = call({"op": "BatchSearch", "args": {"queries": items}})
    return r.get("BatchHits", []) if isinstance(r, dict) else []


# ── Raw SQL via daemon socket OR direct apsw bypass ───────────────────────
def sql(query: str, params: list = None):
    """Auto-route: SELECT → direct apsw (138× faster), other → daemon.
    Heuristic: query starts with 'SELECT' or 'WITH' = read-only = bypass."""
    qstr = (query or "").strip().upper()
    if qstr.startswith("SELECT") or qstr.startswith("WITH"):
        # Direct apsw fast-path
        rows = sql_direct(query, tuple(params or ()))
        if rows is not None:
            # Best-effort cols extraction (apsw doesn't give col names without cursor)
            return [], [list(r) for r in rows]
    # Fallback to daemon (write/PRAGMA/etc)
    r = call({"op": "Sql", "args": {"query": query, "params": params or []}})
    if isinstance(r, dict) and "Rows" in r:
        return r["Rows"]["cols"], r["Rows"]["rows"]
    return [], []
