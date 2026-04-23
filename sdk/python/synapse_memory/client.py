"""Synapse unix-socket client. Length-prefixed msgpack protocol."""
import socket
import struct
from typing import Any, Optional, Union, List, Dict

import msgpack

DEFAULT_SOCK = "/tmp/synapse.sock"


class SynapseError(RuntimeError):
    pass


class Client:
    """Synapse daemon client via unix socket.

    Args:
        sock_path: Path to unix socket (default: /tmp/synapse.sock)
        timeout: Per-call timeout seconds (default: 30, extended for batch)

    Measured latencies (M4 Max, BGE-small ONNX, 2026-04-20):
        ping   p50 58µs
        hybrid p50 8.2ms
        put    p50 335ms (fresh embed) | <1ms (cache hit)
    """

    def __init__(self, sock_path: str = DEFAULT_SOCK, timeout: float = 30.0):
        self.sock_path = sock_path
        self.timeout = timeout

    def _call(self, req: Dict[str, Any], timeout: Optional[float] = None) -> Any:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(timeout or self.timeout)
        try:
            s.connect(self.sock_path)
        except (ConnectionRefusedError, FileNotFoundError) as e:
            raise SynapseError(f"daemon not running at {self.sock_path}: {e}") from e
        try:
            body = msgpack.packb(req)
            s.sendall(struct.pack("<I", len(body)) + body)
            hdr = self._recv_n(s, 4)
            n = struct.unpack("<I", hdr)[0]
            buf = self._recv_n(s, n)
            resp = msgpack.unpackb(buf, raw=False)
        finally:
            s.close()
        if isinstance(resp, dict) and "Err" in resp:
            raise SynapseError(resp["Err"])
        return resp

    @staticmethod
    def _recv_n(s: socket.socket, n: int) -> bytes:
        buf = b""
        while len(buf) < n:
            chunk = s.recv(n - len(buf))
            if not chunk:
                raise SynapseError("connection closed")
            buf += chunk
        return buf

    # --- API ---

    def ping(self) -> bool:
        return self._call({"op": "Ping"}) == "Pong"

    def stats(self) -> Dict[str, int]:
        r = self._call({"op": "Stats"})
        return r.get("Stats", r)

    def put(self, text: str, title: Optional[str] = None,
            uri: Optional[str] = None, meta: Optional[Dict] = None,
            embed: bool = True) -> int:
        req = {"op": "Put", "args": {"title": title, "uri": uri,
                                      "text": text, "meta": meta, "embed": embed}}
        r = self._call(req)
        return r.get("Id", r)

    def put_batch(self, items: List[Dict[str, Any]],
                  embed: bool = True, timeout: float = 600.0) -> List[int]:
        """Bulk insert. Each item: {text, title?, uri?, meta?}.

        10-100× faster than per-item put for unique content.
        Hits embedding cache for duplicates (~707k docs/s cache-hit).
        """
        batch = [{"title": it.get("title"), "uri": it.get("uri"),
                  "text": it["text"], "meta": it.get("meta"), "embed": embed}
                 for it in items]
        r = self._call({"op": "PutBatch", "args": batch}, timeout=timeout)
        return r.get("Ids", r)

    def search(self, query: str, mode: str = "hybrid", limit: int = 10,
               embed_query: bool = True) -> List[Dict[str, Any]]:
        """Search memory.

        Args:
            mode: "hybrid" (BM25+vec RRF) | "lex" (FTS5) | "vec" (kNN)
        """
        mode_map = {"hybrid": "Hybrid", "lex": "Lex", "vec": "Vec"}
        m = mode_map.get(mode.lower(), "Hybrid")
        r = self._call({"op": "Search",
                        "args": {"mode": m, "q": query, "limit": int(limit),
                                 "embed_query": embed_query}})
        return r.get("Hits", r) or []

    def timeline(self, limit: int = 50, offset: int = 0) -> List[Dict]:
        r = self._call({"op": "Timeline", "args": {"limit": limit, "offset": offset}})
        return r.get("Docs", r) or []

    def snap(self, out: str, level: int = 3) -> None:
        self._call({"op": "Snap", "args": {"out": out, "level": level}})

    def verify(self, doc_id: int, verifying_key: bytes) -> bool:
        if len(verifying_key) != 32:
            raise ValueError("verifying_key must be 32 bytes")
        r = self._call({"op": "Verify", "args": {"id": doc_id,
                                                   "vk": list(verifying_key)}})
        return r == "Ok"

    # --- scoped bank API (Hindsight-style sugar) ---

    def bank(self, bank_id: str) -> "Bank":
        return Bank(self, bank_id)


class Bank:
    """Scoped memory — every op carries bank_id in meta.scope.

    Like Hindsight's bankId, but for synapse.
    """

    def __init__(self, client: Client, bank_id: str):
        self.client = client
        self.bank_id = bank_id

    def _scope(self, extra: Optional[Dict] = None) -> Dict:
        m = {"scope": f"bank/{self.bank_id}"}
        if extra:
            m.update(extra)
        return m

    def retain(self, text: str, title: Optional[str] = None,
               meta: Optional[Dict] = None) -> int:
        return self.client.put(text, title=title, meta=self._scope(meta))

    def recall(self, query: str, limit: int = 5) -> List[Dict]:
        hits = self.client.search(query, mode="hybrid", limit=limit * 3)
        return [h for h in hits
                if (h.get("meta") or {}).get("scope") == f"bank/{self.bank_id}"][:limit]

    def reflect(self, limit: int = 20) -> List[Dict]:
        docs = self.client.timeline(limit=limit * 3)
        return [d for d in docs
                if (d.get("meta") or {}).get("scope") == f"bank/{self.bank_id}"][:limit]
