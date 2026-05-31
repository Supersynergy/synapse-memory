#!/usr/bin/env python3
"""Synapse cross-encoder rerank daemon (Unix socket).

Persistent process: model loaded once, listens on socket, serves rerank
requests until shutdown.

Protocol per connection: same length-prefixed msgpack as rerank-sidecar.
One request → one response → connection closed.

Default socket: /tmp/synapse-rerank.sock
Override with SYNAPSE_RERANK_SOCK.

Default model: jinaai/jina-reranker-v1-tiny-en.
Override with SYNAPSE_RERANKER_MODEL.

Start manually:
  /Users/master/.venvs/synapse-backfill-314/bin/python \
      /Users/master/projects/synapse/scripts/rerank-daemon.py &

Stop: kill the PID, socket auto-cleans on next start.
"""
from __future__ import annotations

import os
import signal
import socket
import struct
import sys
import threading
import time

import msgpack
from fastembed.rerank.cross_encoder import TextCrossEncoder

SOCK_PATH = os.environ.get("SYNAPSE_RERANK_SOCK", "/tmp/synapse-rerank.sock")
MODEL_ID = os.environ.get("SYNAPSE_RERANKER_MODEL", "jinaai/jina-reranker-v1-tiny-en")
IDLE_TTL = float(os.environ.get("SYNAPSE_RERANK_IDLE_TTL", "1800"))  # 30min

_LAST_HIT = time.time()
_LOCK = threading.Lock()


def _read_exact(conn: socket.socket, n: int) -> bytes | None:
    buf = bytearray()
    while len(buf) < n:
        chunk = conn.recv(n - len(buf))
        if not chunk:
            return None
        buf.extend(chunk)
    return bytes(buf)


def _send_msg(conn: socket.socket, obj: dict) -> None:
    body = msgpack.packb(obj, use_bin_type=True)
    conn.sendall(struct.pack(">I", len(body)) + body)


def _handle(conn: socket.socket, reranker: TextCrossEncoder) -> None:
    global _LAST_HIT
    try:
        hdr = _read_exact(conn, 4)
        if hdr is None:
            return
        (n,) = struct.unpack(">I", hdr)
        body = _read_exact(conn, n)
        if body is None:
            return
        req = msgpack.unpackb(body, raw=False)
        query = req.get("query") or ""
        docs = req.get("docs") or []
        if not query or not docs:
            _send_msg(conn, {"scores": []})
            return
        with _LOCK:
            scores = [float(s) for s in reranker.rerank(query, docs)]
        _send_msg(conn, {"scores": scores})
        _LAST_HIT = time.time()
    except Exception as e:  # noqa: BLE001
        try:
            _send_msg(conn, {"error": f"{type(e).__name__}: {e}"})
        except Exception:
            pass
    finally:
        try:
            conn.close()
        except Exception:
            pass


def _idle_killer():
    while True:
        time.sleep(60)
        if time.time() - _LAST_HIT > IDLE_TTL:
            os.kill(os.getpid(), signal.SIGTERM)
            return


def main() -> int:
    if os.path.exists(SOCK_PATH):
        try:
            os.unlink(SOCK_PATH)
        except OSError:
            pass

    print(f"loading {MODEL_ID}...", file=sys.stderr, flush=True)
    t = time.time()
    reranker = TextCrossEncoder(model_name=MODEL_ID)
    print(f"loaded in {time.time() - t:.1f}s, listening on {SOCK_PATH}", file=sys.stderr, flush=True)

    srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    srv.bind(SOCK_PATH)
    os.chmod(SOCK_PATH, 0o600)
    srv.listen(8)

    def _shutdown(signum, frame):
        try:
            srv.close()
        finally:
            if os.path.exists(SOCK_PATH):
                os.unlink(SOCK_PATH)
        sys.exit(0)

    signal.signal(signal.SIGTERM, _shutdown)
    signal.signal(signal.SIGINT, _shutdown)

    threading.Thread(target=_idle_killer, daemon=True).start()

    while True:
        try:
            conn, _ = srv.accept()
        except OSError:
            break
        threading.Thread(target=_handle, args=(conn, reranker), daemon=True).start()

    return 0


if __name__ == "__main__":
    sys.exit(main())
