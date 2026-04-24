"""LangChain `Embeddings` + `VectorStore` adapters for synapse.

Usage:
    from langchain_adapter import SynapseVectorStore
    store = SynapseVectorStore(my_embedder)
    store.add_texts(["hello world", "the quick brown fox"])
    hits = store.similarity_search("greeting", k=5)
"""

from __future__ import annotations
from typing import Iterable, List, Tuple

import synapse


class SynapseVectorStore:
    """Minimal LangChain-compatible vector store backed by synapse.I8Index.

    Accepts any embedder exposing `embed_query(text) -> list[float]` and
    `embed_documents(texts) -> list[list[float]]` (e.g. OpenAIEmbeddings,
    HuggingFaceEmbeddings, fastembed.TextEmbedding).
    """

    def __init__(self, embedder, rerank_candidates: int = 80):
        self.embedder = embedder
        self.rerank_candidates = rerank_candidates
        self._rows: List[Tuple[int, List[float]]] = []
        self._texts: dict[int, str] = {}
        self._i8 = None
        self._hamming = None

    def add_texts(self, texts: Iterable[str]) -> List[int]:
        texts = list(texts)
        vecs = self.embedder.embed_documents(texts)
        ids = []
        for text, vec in zip(texts, vecs):
            doc_id = len(self._rows)
            self._rows.append((doc_id, list(map(float, vec))))
            self._texts[doc_id] = text
            ids.append(doc_id)
        # Rebuild indices lazily; a production impl would append incrementally.
        self._i8 = synapse.I8Index.build(self._rows)
        self._hamming = synapse.HammingIndex.build(self._rows)
        return ids

    def similarity_search(self, query: str, k: int = 10) -> List[Tuple[str, float]]:
        if not self._rows:
            return []
        vec = list(map(float, self.embedder.embed_query(query)))
        # Use two-stage rerank when corpus is large enough.
        if self._hamming is not None and len(self._rows) >= 1_000:
            hits = synapse.rerank(
                self._hamming, self._i8, vec, k=k,
                candidates=max(k * 8, self.rerank_candidates),
            )
        else:
            hits = self._i8.search(vec, k=k)
        return [(self._texts[doc_id], score) for doc_id, score in hits]


# --- minimal LangChain Embeddings interface --------------------------------

class LangChainEmbeddingsProtocol:
    """Duck-type marker. Any object with these two methods is accepted."""
    def embed_query(self, text: str) -> List[float]:
        raise NotImplementedError

    def embed_documents(self, texts: List[str]) -> List[List[float]]:
        raise NotImplementedError


if __name__ == "__main__":
    # Demo with a fake embedder — replace with OpenAIEmbeddings, etc.
    class HashEmbedder:
        def __init__(self, dim: int = 128):
            self.dim = dim

        def _hash(self, s: str) -> List[float]:
            import math
            h = hash(s)
            return [math.sin(h + i) for i in range(self.dim)]

        def embed_query(self, text: str) -> List[float]:
            return self._hash(text)

        def embed_documents(self, texts: List[str]) -> List[List[float]]:
            return [self._hash(t) for t in texts]

    emb = HashEmbedder()
    store = SynapseVectorStore(emb)
    store.add_texts(["hello world", "goodbye cruel world", "the quick brown fox"])
    print(store.similarity_search("hello", k=2))
