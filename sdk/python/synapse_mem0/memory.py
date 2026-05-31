"""mem0-compatible Memory class backed by synapse daemon via unix socket."""
from __future__ import annotations
import socket
import struct
import os
import json
import re
import time
import uuid
from typing import Any

import msgpack


_DEFAULT_SOCK = "/tmp/synapse.sock"


class _Transport:
    def __init__(self, sock_path: str, api_key: str | None = None):
        self._path = sock_path
        self._api_key = api_key if api_key is not None else os.environ.get("SYNAPSE_API_KEY")
        self._sock: socket.socket | None = None

    def _connect(self) -> None:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.connect(self._path)
        self._sock = s
        if self._api_key:
            auth = self._roundtrip({"op": "Auth", "args": {"token": self._api_key}})
            if isinstance(auth, dict) and "Err" in auth:
                raise RuntimeError(auth["Err"])

    def _ensure(self) -> None:
        if self._sock is None:
            self._connect()

    def call(self, req: dict) -> Any:
        self._ensure()
        assert self._sock is not None
        resp = self._roundtrip(req)
        if isinstance(resp, dict) and "Err" in resp:
            raise RuntimeError(resp["Err"])
        return resp

    def _roundtrip(self, req: dict) -> Any:
        assert self._sock is not None
        body = msgpack.packb(req)
        self._sock.sendall(struct.pack("<I", len(body)) + body)
        head = self._recv(4)
        n = struct.unpack("<I", head)[0]
        return msgpack.unpackb(self._recv(n), raw=False)

    def _recv(self, n: int) -> bytes:
        buf = b""
        while len(buf) < n:
            chunk = self._sock.recv(n - len(buf))  # type: ignore[union-attr]
            if not chunk:
                raise IOError("eof")
            buf += chunk
        return buf

    def close(self) -> None:
        if self._sock:
            self._sock.close()
            self._sock = None


def _user_prefix(user_id: str) -> str:
    return f"user/{user_id}/"


def _make_title(user_id: str, memory_id: str) -> str:
    return f"{_user_prefix(user_id)}{memory_id}"


def _extract_memory_id(title: str) -> str:
    """Return the memory_id part after the last '/'."""
    return title.rsplit("/", 1)[-1]


def _parse_meta(value: Any) -> dict:
    if isinstance(value, dict):
        return value
    if isinstance(value, str) and value:
        try:
            parsed = json.loads(value)
        except json.JSONDecodeError:
            return {}
        return parsed if isinstance(parsed, dict) else {}
    return {}


def _query_terms(query: str) -> list[str]:
    terms = []
    seen = set()
    for term in re.findall(r"[A-Za-z0-9_]+", query.lower()):
        if len(term) < 2 or term in seen:
            continue
        seen.add(term)
        terms.append(term)
    return terms


def _hit_blob(hit: dict) -> str:
    return " ".join(
        str(hit.get(key) or "")
        for key in ("memory", "text", "content", "title", "id")
    )


def _rank_hits(query: str, hits: list[dict], limit: int) -> list[dict]:
    terms = _query_terms(query)
    unique: list[dict] = []
    seen = set()
    for hit in hits:
        key = str(hit.get("id", _hit_blob(hit)))
        if key in seen:
            continue
        seen.add(key)
        unique.append(hit)

    def score(hit: dict) -> tuple[int, int, float]:
        blob = _hit_blob(hit).lower()
        exact = 1 if query.lower() in blob else 0
        overlap = sum(1 for term in terms if term in blob)
        return (exact, overlap, float(hit.get("score") or 0.0))

    return sorted(unique, key=score, reverse=True)[:limit]


class Memory:
    """Drop-in replacement for mem0.Memory / mem0.MemoryClient."""

    def __init__(self, sock_path: str | None = None, api_key: str | None = None):
        self._t = _Transport(sock_path or os.environ.get("SYNAPSE_SOCK", _DEFAULT_SOCK), api_key=api_key)

    # ------------------------------------------------------------------
    # Core mem0 API
    # ------------------------------------------------------------------

    def add(
        self,
        messages: list[dict] | str,
        user_id: str = "default",
        metadata: dict | None = None,
        **_: Any,
    ) -> dict:
        """Add messages to memory. Returns {'results': [{'id': ..., 'memory': ..., 'event': 'ADD'}]}."""
        if isinstance(messages, str):
            text = messages
        else:
            text = "\n".join(
                m.get("content", "") for m in messages if isinstance(m, dict)
            )
        memory_id = str(uuid.uuid4())
        title = _make_title(user_id, memory_id)
        meta: dict = {"user_id": user_id, "memory_id": memory_id, "created_at": time.time()}
        if metadata:
            meta.update(metadata)
        resp = self._t.call({
            "op": "Put",
            "args": {
                "title": title,
                "uri": None,
                "text": text,
                "meta": meta,
                "embed": False,
            },
        })
        doc_id = resp if isinstance(resp, int) else resp.get("id", memory_id)
        return {
            "results": [{"id": memory_id, "memory": text, "event": "ADD"}],
            "_synapse_doc_id": doc_id,
        }

    def search(
        self,
        query: str,
        user_id: str = "default",
        limit: int = 10,
        **_: Any,
    ) -> dict:
        """Search memories for a user. Returns {'results': [{'id', 'memory', 'score'}]}."""
        scoped = self._search_user_daemon(query, user_id, max(limit * 10, limit))
        if scoped:
            return {"results": _rank_hits(query, scoped, limit)}

        scoped = self._search_user_sql(query, user_id, max(limit * 10, limit))
        if scoped:
            return {"results": _rank_hits(query, scoped, limit)}

        prefix = _user_prefix(user_id)
        resp = self._t.call({
            "op": "Search",
            "args": {"mode": "Lex", "q": query, "limit": limit * 20, "embed_query": False},
        })
        hits = resp if isinstance(resp, list) else resp.get("hits", [])
        results = []
        for h in hits:
            title = h.get("title", "")
            if not title.startswith(prefix):
                continue
            memory_id = _extract_memory_id(title)
            results.append({
                "id": memory_id,
                "memory": h.get("text", ""),
                "score": h.get("score", 0.0),
                "metadata": h.get("meta", {}),
            })
            if len(results) >= limit:
                break
        return {"results": _rank_hits(query, results, limit)}

    def _search_user_daemon(self, query: str, user_id: str, limit: int) -> list[dict]:
        try:
            resp = self._t.call({
                "op": "SearchScoped",
                "args": {
                    "mode": "Lex",
                    "q": query,
                    "limit": int(limit),
                    "embed_query": False,
                    "scope_key": "user_id",
                    "scope_value": user_id,
                    "candidate_limit": int(limit),
                },
            })
        except Exception:
            return []
        hits = resp.get("Hits", resp) if isinstance(resp, dict) else resp
        if not isinstance(hits, list):
            return []
        out = []
        for h in hits:
            if not isinstance(h, dict):
                continue
            meta = _parse_meta(h.get("meta"))
            title = str(h.get("title") or "")
            out.append({
                "id": meta.get("memory_id") or _extract_memory_id(title),
                "memory": h.get("text") or "",
                "score": h.get("score", 0.0),
                "metadata": meta,
            })
        return out

    def _search_user_sql(self, query: str, user_id: str, limit: int) -> list[dict]:
        """Scope by mem0 user before ranking so large global brains do not hide hits."""
        terms = _query_terms(query)[:16]
        filters = []
        params: list[Any] = [user_id]
        for term in terms:
            filters.append("(lower(coalesce(title,'')) LIKE ? OR lower(coalesce(text,'')) LIKE ?)")
            needle = f"%{term}%"
            params.extend([needle, needle])
        if not filters:
            return []
        params.append(int(limit))
        try:
            resp = self._t.call({
                "op": "Sql",
                "args": {
                    "query": (
                        "SELECT id, title, substr(text,1,4000) AS text, meta "
                        "FROM docs "
                        "WHERE meta IS NOT NULL AND json_valid(meta) "
                        "AND json_extract(meta, '$.user_id') = ? "
                        f"AND ({' OR '.join(filters)}) "
                        "ORDER BY id DESC LIMIT ?"
                    ),
                    "params": params,
                },
            })
        except Exception:
            return []

        payload = resp.get("Rows", resp) if isinstance(resp, dict) else {}
        if not isinstance(payload, dict):
            return []
        cols = payload.get("cols") or []
        rows = payload.get("rows") or []
        out = []
        for row in rows:
            doc = dict(zip(cols, row))
            meta = _parse_meta(doc.get("meta"))
            if meta.get("user_id") != user_id:
                continue
            title = str(doc.get("title") or "")
            out.append({
                "id": meta.get("memory_id") or _extract_memory_id(title),
                "memory": doc.get("text") or "",
                "score": 0.0,
                "metadata": meta,
            })
        return out

    def get_all(self, user_id: str = "default", **_: Any) -> dict:
        """Return all memories for a user."""
        scoped = self._all_user_sql(user_id)
        if scoped:
            return {"results": scoped}

        prefix = _user_prefix(user_id)
        resp = self._t.call({
            "op": "Search",
            "args": {"mode": "Lex", "q": prefix, "limit": 1000, "embed_query": False},
        })
        hits = resp if isinstance(resp, list) else resp.get("hits", [])
        results = []
        for h in hits:
            title = h.get("title", "")
            if not title.startswith(prefix):
                continue
            memory_id = _extract_memory_id(title)
            results.append({
                "id": memory_id,
                "memory": h.get("text", ""),
                "metadata": h.get("meta", {}),
            })
        return {"results": results}

    def _all_user_sql(self, user_id: str, limit: int = 1000) -> list[dict]:
        try:
            resp = self._t.call({
                "op": "Sql",
                "args": {
                    "query": (
                        "SELECT id, title, substr(text,1,4000) AS text, meta "
                        "FROM docs "
                        "WHERE meta IS NOT NULL AND json_valid(meta) "
                        "AND json_extract(meta, '$.user_id') = ? "
                        "ORDER BY id DESC LIMIT ?"
                    ),
                    "params": [user_id, int(limit)],
                },
            })
        except Exception:
            return []

        payload = resp.get("Rows", resp) if isinstance(resp, dict) else {}
        if not isinstance(payload, dict):
            return []
        cols = payload.get("cols") or []
        rows = payload.get("rows") or []
        out = []
        for row in rows:
            doc = dict(zip(cols, row))
            meta = _parse_meta(doc.get("meta"))
            if meta.get("user_id") != user_id:
                continue
            title = str(doc.get("title") or "")
            out.append({
                "id": meta.get("memory_id") or _extract_memory_id(title),
                "memory": doc.get("text") or "",
                "metadata": meta,
            })
        return out

    def get(self, memory_id: str, user_id: str = "default", **_: Any) -> dict | None:
        """Get a single memory by id."""
        res = self.get_all(user_id)
        for r in res["results"]:
            if r["id"] == memory_id:
                return r
        return None

    def update(self, memory_id: str, data: str, user_id: str = "default", **_: Any) -> dict:
        """Update (overwrite) a memory's text content."""
        title = _make_title(user_id, memory_id)
        meta = {"user_id": user_id, "memory_id": memory_id, "updated_at": time.time()}
        resp = self._t.call({
            "op": "Put",
            "args": {
                "title": title,
                "uri": None,
                "text": data,
                "meta": meta,
                "embed": False,
            },
        })
        doc_id = resp if isinstance(resp, int) else resp.get("id", memory_id)
        return {"id": memory_id, "memory": data, "event": "UPDATE", "_synapse_doc_id": doc_id}

    def delete(self, memory_id: str, user_id: str = "default", **_: Any) -> dict:
        """Delete a memory by id. Returns {'message': 'Memory deleted successfully!'}."""
        title = _make_title(user_id, memory_id)
        # synapse Delete op by title-prefix search then delete by doc_id
        resp = self._t.call({
            "op": "Search",
            "args": {"mode": "Lex", "q": title, "limit": 5, "embed_query": False},
        })
        hits = resp if isinstance(resp, list) else resp.get("hits", [])
        for h in hits:
            if h.get("title", "") == title:
                doc_id = h.get("id")
                if doc_id is not None:
                    self._t.call({"op": "Delete", "args": {"id": doc_id}})
                break
        return {"message": "Memory deleted successfully!"}

    def delete_all(self, user_id: str = "default", **_: Any) -> dict:
        """Delete all memories for a user."""
        res = self.get_all(user_id)
        for r in res["results"]:
            self.delete(r["id"], user_id)
        return {"message": f"Deleted {len(res['results'])} memories for user '{user_id}'."}

    def history(self, memory_id: str, **_: Any) -> dict:
        """Return history stub — synapse is append-on-update, no version log exposed."""
        return {"results": []}

    def reset(self) -> None:
        """No-op reset (synapse is persistent)."""

    def close(self) -> None:
        self._t.close()

    def __enter__(self) -> "Memory":
        return self

    def __exit__(self, *_: Any) -> None:
        self.close()


# Alias used by mem0 cloud client
MemoryClient = Memory
