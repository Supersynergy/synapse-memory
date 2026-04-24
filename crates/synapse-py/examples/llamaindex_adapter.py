"""LlamaIndex `VectorStore` adapter backed by synapse.

Minimal `BasePydanticVectorStore`-compatible surface: `add`, `query`, `delete`.
"""

from __future__ import annotations
from typing import Any, List, Tuple
import uuid

import synapse


class SynapseLlamaVectorStore:
    """Pass an embed_model at construction or compute embeddings outside."""

    def __init__(self, rerank_threshold: int = 1_000):
        self.rerank_threshold = rerank_threshold
        self._rows: List[Tuple[int, List[float]]] = []
        self._nodes: dict[int, dict[str, Any]] = {}
        self._str_to_int: dict[str, int] = {}
        self._int_to_str: dict[int, str] = {}
        self._i8 = None
        self._hamming = None

    def add(self, nodes: list) -> List[str]:
        """`nodes` is a list of objects with `.node_id`, `.embedding`, `.text`."""
        out_ids = []
        for node in nodes:
            sid = getattr(node, "node_id", None) or str(uuid.uuid4())
            iid = len(self._str_to_int)
            self._str_to_int[sid] = iid
            self._int_to_str[iid] = sid
            self._rows.append((iid, list(map(float, node.embedding))))
            self._nodes[iid] = {"text": getattr(node, "text", ""), "metadata": getattr(node, "metadata", {})}
            out_ids.append(sid)
        self._i8 = synapse.I8Index.build(self._rows)
        self._hamming = synapse.HammingIndex.build(self._rows)
        return out_ids

    def query(self, query_embedding: List[float], similarity_top_k: int = 10):
        if not self._rows or self._i8 is None:
            return {"ids": [], "similarities": [], "nodes": []}
        q = list(map(float, query_embedding))
        if self._hamming is not None and len(self._rows) >= self.rerank_threshold:
            hits = synapse.rerank(
                self._hamming, self._i8, q,
                k=similarity_top_k,
                candidates=max(similarity_top_k * 8, 80),
            )
        else:
            hits = self._i8.search(q, k=similarity_top_k)
        return {
            "ids":          [self._int_to_str[iid] for iid, _ in hits],
            "similarities": [score for _, score in hits],
            "nodes":        [self._nodes[iid] for iid, _ in hits],
        }

    def delete(self, ref_doc_id: str) -> None:
        if ref_doc_id not in self._str_to_int:
            return
        iid = self._str_to_int.pop(ref_doc_id)
        self._int_to_str.pop(iid, None)
        self._nodes.pop(iid, None)
        self._rows = [(i, v) for i, v in self._rows if i != iid]
        if self._rows:
            self._i8 = synapse.I8Index.build(self._rows)
            self._hamming = synapse.HammingIndex.build(self._rows)
        else:
            self._i8 = None
            self._hamming = None
