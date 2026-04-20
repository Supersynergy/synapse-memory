"""
SynapseRetriever — wraps synapse search() and emits a Langfuse span per call.
Interface spec: https://langfuse.com/docs/tracing (Langfuse Python SDK v3)
"""
from __future__ import annotations
import socket
import struct
import time
from typing import Any

import msgpack


_DEFAULT_SOCK = "/tmp/synapse.sock"


class _Transport:
    def __init__(self, sock_path: str):
        self._path = sock_path
        self._sock: socket.socket | None = None

    def _ensure(self) -> None:
        if self._sock is None:
            s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            s.connect(self._path)
            self._sock = s

    def call(self, req: dict) -> Any:
        self._ensure()
        body = msgpack.packb(req)
        self._sock.sendall(struct.pack("<I", len(body)) + body)  # type: ignore[union-attr]
        head = self._recv_n(4)
        n = struct.unpack("<I", head)[0]
        return msgpack.unpackb(self._recv_n(n), raw=False)

    def _recv_n(self, n: int) -> bytes:
        buf = b""
        while len(buf) < n:
            chunk = self._sock.recv(n - len(buf))  # type: ignore[union-attr]
            if not chunk:
                raise ConnectionError("synapsed closed connection")
            buf += chunk
        return buf

    def close(self) -> None:
        if self._sock:
            self._sock.close()
            self._sock = None


class SynapseRetriever:
    """
    Drop-in retriever that logs each search as a Langfuse `retrieval` span.

    Usage::

        from langfuse import Langfuse
        from synapse_langfuse import SynapseRetriever

        lf = Langfuse()
        retriever = SynapseRetriever(langfuse=lf)
        hits = retriever.search("capital of France", trace_id="t-001")
    """

    def __init__(
        self,
        sock_path: str = _DEFAULT_SOCK,
        mode: str = "Hybrid",
        langfuse: Any = None,
    ):
        self._transport = _Transport(sock_path)
        self._mode = mode
        self._langfuse = langfuse

    def search(
        self,
        query: str,
        limit: int = 10,
        trace_id: str | None = None,
    ) -> list[dict]:
        t0 = time.perf_counter()
        resp = self._transport.call({
            "op": "Search",
            "args": {
                "mode": self._mode,
                "q": query,
                "limit": limit,
                "embed_query": self._mode in ("Vec", "Hybrid"),
            },
        })
        latency_ms = (time.perf_counter() - t0) * 1000
        hits: list[dict] = resp.get("Hits", []) if isinstance(resp, dict) else []

        if self._langfuse is not None:
            self._emit_span(query, hits, latency_ms, trace_id)

        return hits

    def _emit_span(
        self, query: str, hits: list[dict], latency_ms: float, trace_id: str | None
    ) -> None:
        """Emit a Langfuse span. Silently no-ops if langfuse is unavailable."""
        try:
            trace = (
                self._langfuse.trace(id=trace_id)
                if trace_id
                else self._langfuse.trace()
            )
            trace.span(
                name="synapse-retrieval",
                input={"query": query},
                output={"hits": len(hits), "top_score": hits[0].get("score") if hits else None},
                metadata={"latency_ms": round(latency_ms, 2), "mode": self._mode},
            )
        except Exception:
            pass

    def close(self) -> None:
        self._transport.close()
