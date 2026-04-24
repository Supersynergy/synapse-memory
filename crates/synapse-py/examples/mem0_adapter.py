"""Mem0-compatible vector-store adapter backed by synapse.

Mem0 calls `VectorStore.insert(vectors, payloads, ids)` and
`VectorStore.search(query, limit, filters)`. The implementation below wires
those to synapse.I8Index + HammingIndex for sub-ms retrieval.

Usage:
    from mem0_adapter import SynapseMem0Store
    store = SynapseMem0Store(collection_name="default")
    store.insert(vectors=[[0.1]*384], payloads=[{"text": "hello"}], ids=["m1"])
    hits = store.search(query=[0.1]*384, limit=5)
"""

from __future__ import annotations
from typing import Any, Iterable, List, Tuple

import synapse


class SynapseMem0Store:
    """Drop-in `VectorStoreBase`-compatible adapter."""

    def __init__(self, collection_name: str = "default", rerank: bool = True):
        self.collection_name = collection_name
        self.rerank_on = rerank
        # mem0 uses string ids; synapse uses i64 — we keep a map.
        self._str_to_int: dict[str, int] = {}
        self._int_to_str: dict[int, str] = {}
        self._payloads: dict[str, dict[str, Any]] = {}
        self._rows: List[Tuple[int, List[float]]] = []
        self._i8 = None
        self._hamming = None

    def _intern(self, sid: str) -> int:
        if sid not in self._str_to_int:
            i = len(self._str_to_int)
            self._str_to_int[sid] = i
            self._int_to_str[i] = sid
        return self._str_to_int[sid]

    def insert(
        self,
        vectors: Iterable[List[float]],
        payloads: Iterable[dict[str, Any]],
        ids: Iterable[str],
    ) -> None:
        for vec, payload, sid in zip(vectors, payloads, ids):
            iid = self._intern(sid)
            self._rows.append((iid, list(map(float, vec))))
            self._payloads[sid] = payload
        # Rebuild indices
        self._i8 = synapse.I8Index.build(self._rows)
        self._hamming = synapse.HammingIndex.build(self._rows)

    def search(
        self,
        query: List[float],
        limit: int = 5,
        filters: dict[str, Any] | None = None,   # noqa: ARG002 — mem0 signature parity
    ) -> List[dict[str, Any]]:
        if not self._rows or self._i8 is None:
            return []
        q = list(map(float, query))
        if self.rerank_on and self._hamming is not None and len(self._rows) >= 1_000:
            hits = synapse.rerank(self._hamming, self._i8, q, k=limit, candidates=max(limit * 8, 80))
        else:
            hits = self._i8.search(q, k=limit)
        return [
            {
                "id": self._int_to_str[iid],
                "score": score,
                "payload": self._payloads.get(self._int_to_str[iid], {}),
            }
            for iid, score in hits
        ]

    def delete(self, ids: Iterable[str]) -> None:
        kill = set(ids)
        self._rows = [(iid, v) for iid, v in self._rows if self._int_to_str[iid] not in kill]
        for sid in kill:
            self._payloads.pop(sid, None)
            if sid in self._str_to_int:
                iid = self._str_to_int.pop(sid)
                self._int_to_str.pop(iid, None)
        if self._rows:
            self._i8 = synapse.I8Index.build(self._rows)
            self._hamming = synapse.HammingIndex.build(self._rows)
        else:
            self._i8 = None
            self._hamming = None
