"""
SynapseMemoryPlugin — browser-use on_page_visit hook → synapse put.
Interface spec: https://github.com/browser-use/browser-use (84k★)
Hook point: BrowserAgent callbacks / on_step_end
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
                raise ConnectionError("synapsed closed")
            buf += chunk
        return buf

    def close(self) -> None:
        if self._sock:
            self._sock.close()
            self._sock = None


class SynapseMemoryPlugin:
    """
    Memory plugin for browser-use agents.

    Wire into browser-use::

        from browser_use import Agent
        from synapse_browseruse import SynapseMemoryPlugin

        plugin = SynapseMemoryPlugin()
        agent = Agent(on_page_visit=plugin.on_page_visit, ...)

    Or use as a standalone callback in any browsing loop.
    """

    def __init__(self, sock_path: str = _DEFAULT_SOCK, embed: bool = True):
        self._transport = _Transport(sock_path)
        self._embed = embed

    def on_page_visit(self, url: str, title: str = "", content: str = "") -> None:
        """Call on every page visit. Stores URL + title + content to synapse."""
        text = f"{title}\n{content}".strip() if (title or content) else url
        self._transport.call({
            "op": "Put",
            "args": {
                "text": text,
                "uri": url,
                "title": title or None,
                "meta": {"visited_at": int(time.time()), "type": "browse"},
                "embed": self._embed,
            },
        })

    def on_step_end(self, step: Any) -> None:
        """
        browser-use AgentHistoryList step hook.
        Extracts URL/title from step result if available.
        """
        try:
            result = step.result if hasattr(step, "result") else step
            url = getattr(result, "url", None) or (result.get("url") if isinstance(result, dict) else None)
            title = getattr(result, "title", "") or (result.get("title", "") if isinstance(result, dict) else "")
            content = getattr(result, "extracted_content", "") or ""
            if url:
                self.on_page_visit(url, title=title, content=content)
        except Exception:
            pass

    def recall(self, query: str, limit: int = 10) -> list[dict]:
        """Search browse history by semantic query."""
        resp = self._transport.call({
            "op": "Search",
            "args": {
                "mode": "Hybrid",
                "q": query,
                "limit": limit,
                "embed_query": True,
            },
        })
        hits = resp.get("Hits", []) if isinstance(resp, dict) else []
        return [h for h in hits if isinstance(h.get("meta"), dict) and h["meta"].get("type") == "browse"]

    def close(self) -> None:
        self._transport.close()
