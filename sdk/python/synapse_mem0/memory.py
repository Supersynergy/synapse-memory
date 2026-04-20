"""mem0-compatible Memory class backed by synapse daemon via unix socket."""
from __future__ import annotations
import socket
import struct
import time
import uuid
from typing import Any

import msgpack


_DEFAULT_SOCK = "/tmp/synapse.sock"


class _Transport:
    def __init__(self, sock_path: str):
        self._path = sock_path
        self._sock: socket.socket | None = None

    def _connect(self) -> None:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.connect(self._path)
        self._sock = s

    def _ensure(self) -> None:
        if self._sock is None:
            self._connect()

    def call(self, req: dict) -> Any:
        self._ensure()
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


class Memory:
    """Drop-in replacement for mem0.Memory / mem0.MemoryClient."""

    def __init__(self, sock_path: str = _DEFAULT_SOCK):
        self._t = _Transport(sock_path)

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
        prefix = _user_prefix(user_id)
        resp = self._t.call({
            "op": "Search",
            "args": {"mode": "Lex", "q": query, "limit": limit * 3, "embed_query": False},
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
        return {"results": results}

    def get_all(self, user_id: str = "default", **_: Any) -> dict:
        """Return all memories for a user."""
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
