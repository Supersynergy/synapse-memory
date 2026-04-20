"""
SynapsePromptfooProvider — plugs synapse into promptfoo as a custom provider.
Interface spec: https://www.promptfoo.dev/docs/providers/custom-api
"""
from __future__ import annotations
import json
import socket
import struct
from typing import Any

import msgpack


_DEFAULT_SOCK = "/tmp/synapse.sock"


class SynapsePromptfooProvider:
    """
    Custom promptfoo provider that queries synapse and returns top-k passages.
    Invoke from promptfoo YAML: provider: python:synapse_promptfoo.provider:call
    """

    def __init__(self, sock_path: str = _DEFAULT_SOCK, mode: str = "Hybrid", limit: int = 5):
        self._sock_path = sock_path
        self._mode = mode
        self._limit = limit

    def call_api(self, prompt: str, options: dict | None = None) -> dict:
        """promptfoo calls this. Returns {"output": str, "tokenUsage": {...}}."""
        opts = options or {}
        limit = opts.get("limit", self._limit)

        hits = self._search(prompt, limit)
        if not hits:
            return {"output": "No relevant context found.", "tokenUsage": {"total": 0}}

        formatted = "\n\n".join(
            f"[{i+1}] (score={h.get('score', 0):.3f}) {h.get('text', '')}"
            for i, h in enumerate(hits)
        )
        return {
            "output": formatted,
            "tokenUsage": {"total": sum(len(h.get("text", "").split()) for h in hits)},
        }

    def _search(self, query: str, limit: int) -> list[dict]:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            s.connect(self._sock_path)
            body = msgpack.packb({
                "op": "Search",
                "args": {
                    "mode": self._mode,
                    "q": query,
                    "limit": limit,
                    "embed_query": self._mode in ("Vec", "Hybrid"),
                },
            })
            s.sendall(struct.pack("<I", len(body)) + body)
            head = self._recv_n(s, 4)
            n = struct.unpack("<I", head)[0]
            resp = msgpack.unpackb(self._recv_n(s, n), raw=False)
            return resp.get("Hits", []) if isinstance(resp, dict) else []
        finally:
            s.close()

    @staticmethod
    def _recv_n(s: socket.socket, n: int) -> bytes:
        buf = b""
        while len(buf) < n:
            chunk = s.recv(n - len(buf))
            if not chunk:
                raise ConnectionError("synapsed closed")
            buf += chunk
        return buf


# promptfoo entry point: python:synapse_promptfoo.provider:call
def call(prompt: str, options: dict | None = None, context: dict | None = None) -> dict:
    provider = SynapsePromptfooProvider(
        sock_path=(options or {}).get("sock_path", _DEFAULT_SOCK),
        mode=(options or {}).get("mode", "Hybrid"),
        limit=int((options or {}).get("limit", 5)),
    )
    return provider.call_api(prompt, options)
