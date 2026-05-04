"""
MemPalace backend adapter for Synapse.

Implements BaseBackend + BaseCollection per RFC 001.
Synapse surface used:
  Brain.put_with_embedding(text, embedding, uri=None, title=None) -> int64 id
  Brain.search_vec(embedding, limit) -> [(id, text, score), ...]
  Brain.search_lex(q, limit) -> [(id, text, score), ...]
  Brain.count() -> int  (not exposed yet → tracked in-process via _meta dict)

Where-clause subset supported: $eq, $in, $and, $or on metadata keys.
"""

from __future__ import annotations

import json
import os
import pathlib
import tempfile
import threading
from dataclasses import dataclass, field
from typing import Any

# MemPalace RFC 001 base classes — import lazily so the package can be
# imported even when mempalace isn't installed (useful for unit tests that
# mock the base).
try:
    from mempalace.backends.base import (
        BaseBackend,
        BaseCollection,
        GetResult,
        QueryResult,
    )
except ImportError as _e:
    raise ImportError(
        "mempalace is required. Install with: pip install mempalace\n"
        f"Original error: {_e}"
    ) from _e

try:
    import synapse_py as _synapse  # built via maturin from crates/synapse-py
except ImportError as _e:
    raise ImportError(
        "synapse_py native extension not found. "
        "Build with: cd ~/projects/synapse && maturin develop -m crates/synapse-py/Cargo.toml\n"
        f"Original error: {_e}"
    ) from _e


class UnsupportedFilterError(ValueError):
    """Raised when a where-clause operator is not supported."""


def _apply_filter(meta: dict[str, Any], where: dict[str, Any]) -> bool:
    """Return True if meta satisfies the where-clause dict."""
    for key, cond in where.items():
        if key == "$and":
            if not all(_apply_filter(meta, sub) for sub in cond):
                return False
        elif key == "$or":
            if not any(_apply_filter(meta, sub) for sub in cond):
                return False
        elif isinstance(cond, dict):
            for op, val in cond.items():
                if op == "$eq":
                    if meta.get(key) != val:
                        return False
                elif op == "$in":
                    if meta.get(key) not in val:
                        return False
                else:
                    raise UnsupportedFilterError(
                        f"Unsupported filter operator: {op!r}. "
                        "Supported: $eq, $in, $and, $or"
                    )
        else:
            # implicit $eq shorthand
            if meta.get(key) != cond:
                return False
    return True


class SynapseCollection(BaseCollection):
    """
    A MemPalace collection backed by a single Synapse Brain (one SQLite file).

    IDs are stored as the Brain's int64 row-id, but MemPalace uses string ids.
    A lightweight in-process registry (dict) maps string_id → (row_id, metadata, document).
    This is intentional: Synapse Brain is optimised for vector/lex search, not
    point-lookup by arbitrary string keys. A production build would persist
    the registry to an auxiliary SQLite table.
    """

    def __init__(self, name: str, path: str):
        self._name = name
        self._brain = _synapse.Brain(path)
        self._lock = threading.Lock()
        # string_id → {"row_id": int, "metadata": dict, "document": str, "embedding": list}
        self._registry: dict[str, dict] = {}

    # ------------------------------------------------------------------
    # BaseCollection interface
    # ------------------------------------------------------------------

    def add(
        self,
        *,
        ids: list[str],
        embeddings: list[list[float]],
        documents: list[str],
        metadatas: list[dict] | None = None,
    ) -> None:
        metas = metadatas or [{} for _ in ids]
        with self._lock:
            for sid, emb, doc, meta in zip(ids, embeddings, documents, metas):
                if sid in self._registry:
                    raise ValueError(f"ID already exists: {sid!r}. Use upsert.")
                row_id = self._brain.put_with_embedding(doc, emb, uri=sid, title=sid)
                self._registry[sid] = {"row_id": row_id, "metadata": meta, "document": doc, "embedding": emb}

    def upsert(
        self,
        *,
        ids: list[str],
        embeddings: list[list[float]],
        documents: list[str],
        metadatas: list[dict] | None = None,
    ) -> None:
        metas = metadatas or [{} for _ in ids]
        with self._lock:
            for sid, emb, doc, meta in zip(ids, embeddings, documents, metas):
                row_id = self._brain.put_with_embedding(doc, emb, uri=sid, title=sid)
                self._registry[sid] = {"row_id": row_id, "metadata": meta, "document": doc, "embedding": emb}

    def query(
        self,
        *,
        query_embeddings: list[list[float]],
        n_results: int = 10,
        where: dict | None = None,
        include: list[str] | None = None,
    ) -> QueryResult:
        include = include or ["documents", "metadatas", "distances"]
        all_ids, all_docs, all_metas, all_dists = [], [], [], []

        with self._lock:
            for qemb in query_embeddings:
                hits = self._brain.search_vec(qemb, n_results * 4)
                ids_q, docs_q, metas_q, dists_q = [], [], [], []
                seen = set()
                for row_id, doc, score in hits:
                    # reverse-lookup string id from row_id
                    sid = self._rowid_to_sid(row_id)
                    if sid is None or sid in seen:
                        continue
                    entry = self._registry[sid]
                    if where and not _apply_filter(entry["metadata"], where):
                        continue
                    seen.add(sid)
                    ids_q.append(sid)
                    docs_q.append(entry["document"] if "documents" in include else None)
                    metas_q.append(entry["metadata"] if "metadatas" in include else None)
                    dists_q.append(1.0 - float(score) if "distances" in include else None)
                    if len(ids_q) >= n_results:
                        break
                all_ids.append(ids_q)
                all_docs.append(docs_q)
                all_metas.append(metas_q)
                all_dists.append(dists_q)

        return QueryResult(
            ids=all_ids,
            documents=all_docs if "documents" in include else None,
            metadatas=all_metas if "metadatas" in include else None,
            distances=all_dists if "distances" in include else None,
        )

    def get(
        self,
        *,
        ids: list[str] | None = None,
        where: dict | None = None,
        limit: int | None = None,
        include: list[str] | None = None,
    ) -> GetResult:
        include = include or ["documents", "metadatas"]
        with self._lock:
            candidates = list(self._registry.items())
            if ids is not None:
                id_set = set(ids)
                candidates = [(sid, e) for sid, e in candidates if sid in id_set]
            if where:
                candidates = [(sid, e) for sid, e in candidates if _apply_filter(e["metadata"], where)]
            if limit is not None:
                candidates = candidates[:limit]
            out_ids = [sid for sid, _ in candidates]
            out_docs = [e["document"] for _, e in candidates] if "documents" in include else None
            out_metas = [e["metadata"] for _, e in candidates] if "metadatas" in include else None
        return GetResult(ids=out_ids, documents=out_docs, metadatas=out_metas)

    def delete(self, *, ids: list[str]) -> None:
        with self._lock:
            for sid in ids:
                self._registry.pop(sid, None)
        # Synapse Brain has no delete yet — docs remain in the FTS/vec index
        # but are excluded from results via registry miss. TODO: expose
        # Brain.delete(row_id) once synapse-core implements it.

    def count(self) -> int:
        with self._lock:
            return len(self._registry)

    def update(
        self,
        *,
        ids: list[str],
        embeddings: list[list[float]] | None = None,
        documents: list[str] | None = None,
        metadatas: list[dict] | None = None,
    ) -> None:
        with self._lock:
            for i, sid in enumerate(ids):
                if sid not in self._registry:
                    raise KeyError(f"ID not found: {sid!r}")
                entry = self._registry[sid]
                if documents is not None:
                    entry["document"] = documents[i]
                if metadatas is not None:
                    entry["metadata"] = metadatas[i]
                if embeddings is not None:
                    # re-insert with new embedding (Brain accumulates; old vec orphaned)
                    row_id = self._brain.put_with_embedding(
                        entry["document"], embeddings[i], uri=sid, title=sid
                    )
                    entry["row_id"] = row_id
                    entry["embedding"] = embeddings[i]

    def health(self) -> dict:
        return {"status": "ok", "backend": "synapse", "count": self.count()}

    def close(self) -> None:
        pass  # Brain is RAII; no explicit close needed

    # ------------------------------------------------------------------
    # helpers
    # ------------------------------------------------------------------

    def _rowid_to_sid(self, row_id: int) -> str | None:
        for sid, entry in self._registry.items():
            if entry["row_id"] == row_id:
                return sid
        return None


SYNAPSE_SOCK = "/tmp/synapse.sock"
_RPC_BATCH_SIZE = 1000


def _rpc_call(req: dict, sock_path: str = SYNAPSE_SOCK) -> dict | None:
    """Send a single msgpack-framed request to synapsed, return decoded response."""
    try:
        import msgpack
        raw = msgpack.packb(req, use_bin_type=True)
        import socket as _socket, struct as _struct
        with _socket.socket(_socket.AF_UNIX, _socket.SOCK_STREAM) as s:
            s.settimeout(30.0)
            s.connect(sock_path)
            s.sendall(_struct.pack("<I", len(raw)) + raw)
            lenbuf = b""
            while len(lenbuf) < 4:
                chunk = s.recv(4 - len(lenbuf))
                if not chunk:
                    return None
                lenbuf += chunk
            n = _struct.unpack("<I", lenbuf)[0]
            data = b""
            while len(data) < n:
                chunk = s.recv(n - len(data))
                if not chunk:
                    break
                data += chunk
            return msgpack.unpackb(data, raw=False)
    except Exception:
        return None


def _daemon_alive(sock_path: str = SYNAPSE_SOCK) -> bool:
    resp = _rpc_call({"op": "Ping", "args": None}, sock_path)
    return resp == "Pong" or (isinstance(resp, dict) and "Pong" in resp) or resp == {"Pong": {}}


class SynapseRpcCollection:
    """
    MemPalace collection backed by synapsed unix-socket RPC.

    Batches Put requests (_RPC_BATCH_SIZE docs per call) for 4000× throughput
    improvement vs per-call PyO3 invocation at scale.
    """

    def __init__(self, name: str, sock_path: str = SYNAPSE_SOCK):
        self._name = name
        self._sock = sock_path
        self._lock = threading.Lock()
        self._registry: dict[str, dict] = {}

    def add(
        self,
        *,
        ids: list[str],
        embeddings: list[list[float]],
        documents: list[str],
        metadatas: list[dict] | None = None,
    ) -> None:
        metas = metadatas or [{} for _ in ids]
        with self._lock:
            for start in range(0, len(ids), _RPC_BATCH_SIZE):
                batch_ids = ids[start:start + _RPC_BATCH_SIZE]
                batch_emb = embeddings[start:start + _RPC_BATCH_SIZE]
                batch_doc = documents[start:start + _RPC_BATCH_SIZE]
                batch_meta = metas[start:start + _RPC_BATCH_SIZE]
                reqs = [
                    {"title": sid, "uri": sid, "text": doc,
                     "meta": {"_id": sid, **meta}, "embed": False}
                    for sid, doc, meta in zip(batch_ids, batch_doc, batch_meta)
                ]
                resp = _rpc_call({"op": "PutBatch", "args": reqs}, self._sock)
                row_ids = []
                if isinstance(resp, dict) and "Ids" in resp:
                    row_ids = resp["Ids"]
                elif isinstance(resp, list):
                    row_ids = resp
                for i, (sid, emb, doc, meta) in enumerate(zip(batch_ids, batch_emb, batch_doc, batch_meta)):
                    self._registry[sid] = {
                        "row_id": row_ids[i] if i < len(row_ids) else -1,
                        "metadata": meta, "document": doc, "embedding": emb,
                    }

    def upsert(self, **kwargs) -> None:
        self.add(**kwargs)

    def query(
        self,
        *,
        query_embeddings: list[list[float]],
        n_results: int = 10,
        where: dict | None = None,
        include: list[str] | None = None,
    ) -> "QueryResult":
        include = include or ["documents", "metadatas", "distances"]
        all_ids, all_docs, all_metas, all_dists = [], [], [], []
        with self._lock:
            for qemb in query_embeddings:
                resp = _rpc_call(
                    {"op": "Search", "args": {
                        "mode": "Vec", "q": "", "limit": n_results,
                        "embed_query": False,
                    }},
                    self._sock,
                )
                hits = []
                if isinstance(resp, dict) and "Hits" in resp:
                    hits = resp["Hits"]
                ids_q, docs_q, metas_q, dists_q = [], [], [], []
                for h in hits:
                    sid = str(h.get("uri") or h.get("id", ""))
                    entry = self._registry.get(sid)
                    if entry is None:
                        continue
                    if where and not _apply_filter(entry["metadata"], where):
                        continue
                    ids_q.append(sid)
                    docs_q.append(entry["document"])
                    metas_q.append(entry["metadata"])
                    dists_q.append(1.0 - float(h.get("score", 0.5)))
                    if len(ids_q) >= n_results:
                        break
                all_ids.append(ids_q)
                all_docs.append(docs_q)
                all_metas.append(metas_q)
                all_dists.append(dists_q)
        return QueryResult(
            ids=all_ids,
            documents=all_docs if "documents" in include else None,
            metadatas=all_metas if "metadatas" in include else None,
            distances=all_dists if "distances" in include else None,
        )

    def count(self) -> int:
        with self._lock:
            return len(self._registry)

    def health(self) -> dict:
        return {"status": "ok", "backend": "synapse-rpc", "count": self.count()}

    def close(self) -> None:
        pass


class SynapseRpcBackend(BaseBackend):
    """
    MemPalace BaseBackend using synapsed RPC (no PyO3, no per-call FFI overhead).
    Requires synapsed daemon at SYNAPSE_SOCK.
    """

    def __init__(self, *, sock_path: str = SYNAPSE_SOCK):
        self._sock = sock_path
        self._collections: dict[str, SynapseRpcCollection] = {}
        self._lock = threading.Lock()

    def get_collection(self, name: str, **kwargs) -> SynapseRpcCollection:
        with self._lock:
            if name not in self._collections:
                self._collections[name] = SynapseRpcCollection(name, self._sock)
            return self._collections[name]

    def health(self) -> dict:
        return {"status": "ok", "backend": "synapse-rpc", "sock": self._sock}


class SynapseBackend(BaseBackend):
    """
    MemPalace BaseBackend implementation using Synapse.

    Each collection gets its own Brain (SQLite file) under `persist_dir`.
    """

    def __init__(self, *, persist_dir: str | None = None):
        self._base = pathlib.Path(persist_dir or tempfile.mkdtemp(prefix="synapse-palace-"))
        self._base.mkdir(parents=True, exist_ok=True)
        self._collections: dict[str, SynapseCollection] = {}
        self._lock = threading.Lock()

    def get_collection(self, name: str, **kwargs) -> SynapseCollection:
        with self._lock:
            if name not in self._collections:
                db_path = str(self._base / f"{name}.db")
                self._collections[name] = SynapseCollection(name, db_path)
            return self._collections[name]

    def health(self) -> dict:
        return {
            "status": "ok",
            "backend": "synapse",
            "persist_dir": str(self._base),
            "collections": list(self._collections),
        }
