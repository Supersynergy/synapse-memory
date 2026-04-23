"""
mem0 compatibility tests — run against the shim WITHOUT a real synapse daemon
by using a FakeTransport that records calls.
"""
import socket
import struct
import threading
import time
import uuid
import pytest

from synapsedb_mem0.memory import Memory, _make_title, _user_prefix, _extract_memory_id


# ---------------------------------------------------------------------------
# Fake synapse server (in-process, unix socket)
# ---------------------------------------------------------------------------

import msgpack


class FakeSynapse:
    """Minimal synapse-protocol server backed by an in-memory dict."""

    def __init__(self, sock_path: str):
        self._path = sock_path
        self._docs: dict[int, dict] = {}   # doc_id -> {title, text, meta}
        self._next_id = 1
        self._srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        import os
        if os.path.exists(sock_path):
            os.unlink(sock_path)
        self._srv.bind(sock_path)
        self._srv.listen(5)
        self._thread = threading.Thread(target=self._serve, daemon=True)
        self._thread.start()

    def _serve(self) -> None:
        while True:
            try:
                conn, _ = self._srv.accept()
            except OSError:
                return
            threading.Thread(target=self._handle, args=(conn,), daemon=True).start()

    def _handle(self, conn: socket.socket) -> None:
        try:
            while True:
                head = self._recv(conn, 4)
                if not head:
                    return
                n = struct.unpack("<I", head)[0]
                body = self._recv(conn, n)
                req = msgpack.unpackb(body, raw=False)
                resp = self._dispatch(req)
                out = msgpack.packb(resp)
                conn.sendall(struct.pack("<I", len(out)) + out)
        except Exception:
            pass
        finally:
            conn.close()

    def _recv(self, conn: socket.socket, n: int) -> bytes:
        buf = b""
        while len(buf) < n:
            chunk = conn.recv(n - len(buf))
            if not chunk:
                return buf
            buf += chunk
        return buf

    def _dispatch(self, req: dict) -> object:
        op = req.get("op")
        args = req.get("args", {})
        if op == "Ping":
            return "Pong"
        if op == "Put":
            doc_id = self._next_id
            self._next_id += 1
            self._docs[doc_id] = {
                "id": doc_id,
                "title": args.get("title", ""),
                "text": args.get("text", ""),
                "meta": args.get("meta") or {},
            }
            return doc_id
        if op == "Search":
            q: str = args.get("q", "").lower()
            limit: int = args.get("limit", 10)
            hits = []
            for doc in self._docs.values():
                if q in doc["title"].lower() or q in doc["text"].lower():
                    hits.append({"id": doc["id"], "title": doc["title"],
                                 "text": doc["text"], "meta": doc["meta"], "score": 1.0})
            return hits[:limit]
        if op == "Delete":
            doc_id = args.get("id")
            self._docs.pop(doc_id, None)
            return {"ok": True}
        return {"error": f"unknown op {op}"}

    def stop(self) -> None:
        self._srv.close()


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

SOCK_PATH = "/tmp/synapse_test_shim.sock"


@pytest.fixture(scope="module")
def fake_server():
    srv = FakeSynapse(SOCK_PATH)
    yield srv
    srv.stop()


@pytest.fixture()
def mem(fake_server):
    m = Memory(sock_path=SOCK_PATH)
    yield m
    m.close()


# ---------------------------------------------------------------------------
# Tests — mirrors mem0 quickstart examples
# ---------------------------------------------------------------------------

def test_add_string(mem: Memory):
    result = mem.add("Alice likes hiking on weekends.", user_id="alice")
    assert "results" in result
    assert result["results"][0]["event"] == "ADD"
    assert result["results"][0]["id"]


def test_add_messages(mem: Memory):
    messages = [
        {"role": "user", "content": "I love Python programming."},
        {"role": "assistant", "content": "Great, Python is versatile!"},
    ]
    result = mem.add(messages, user_id="bob")
    assert result["results"][0]["event"] == "ADD"
    assert "Python" in result["results"][0]["memory"]


def test_search_returns_relevant(mem: Memory):
    mem.add("Charlie enjoys playing chess.", user_id="charlie")
    result = mem.search("chess", user_id="charlie")
    assert "results" in result
    texts = [r["memory"] for r in result["results"]]
    assert any("chess" in t for t in texts)


def test_search_user_isolation(mem: Memory):
    mem.add("Dave likes soccer.", user_id="dave")
    mem.add("Eve likes basketball.", user_id="eve")
    dave_results = mem.search("likes", user_id="dave")
    for r in dave_results["results"]:
        assert r["metadata"].get("user_id") == "dave"


def test_get_all(mem: Memory):
    mem.add("Frank reads sci-fi.", user_id="frank")
    mem.add("Frank also codes in Rust.", user_id="frank")
    result = mem.get_all(user_id="frank")
    assert len(result["results"]) >= 2


def test_update(mem: Memory):
    res = mem.add("Grace's hobby is painting.", user_id="grace")
    mid = res["results"][0]["id"]
    upd = mem.update(mid, "Grace's hobby is watercolour painting.", user_id="grace")
    assert upd["event"] == "UPDATE"
    assert "watercolour" in upd["memory"]


def test_delete(mem: Memory):
    res = mem.add("Henry dislikes loud music.", user_id="henry")
    mid = res["results"][0]["id"]
    del_res = mem.delete(mid, user_id="henry")
    assert "deleted" in del_res["message"].lower()


def test_delete_all(mem: Memory):
    mem.add("Iris likes cats.", user_id="iris")
    mem.add("Iris has two dogs.", user_id="iris")
    res = mem.delete_all(user_id="iris")
    assert "iris" in res["message"]


def test_history_returns_stub(mem: Memory):
    res = mem.add("Jack climbs mountains.", user_id="jack")
    mid = res["results"][0]["id"]
    h = mem.history(mid)
    assert "results" in h


def test_context_manager(fake_server):
    with Memory(sock_path=SOCK_PATH) as m:
        r = m.add("Context-manager test", user_id="cm_user")
        assert r["results"][0]["event"] == "ADD"


def test_memory_client_alias():
    from synapsedb_mem0 import MemoryClient
    assert MemoryClient is Memory


def test_helper_funcs():
    assert _user_prefix("alice") == "user/alice/"
    assert _make_title("alice", "abc-123") == "user/alice/abc-123"
    assert _extract_memory_id("user/alice/abc-123") == "abc-123"
