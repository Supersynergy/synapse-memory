#!/usr/bin/env python3
"""Synapse cross-encoder rerank sidecar.

Protocol (length-prefixed msgpack over stdio, mirrors synapse-mlx-embed.py):

  ready:   {"ready": True, "model": "<id>"}
  request: {"query": "...", "docs": ["...", ...]}
  response: {"scores": [f32, ...]}  (higher = more relevant)
            {"error": "msg"}

Each message is 4-byte big-endian length + msgpack body. Model is loaded
once at startup, then amortized across many queries.

Model default: Xenova/ms-marco-MiniLM-L-6-v2 (~22MB, ~5ms/batch on M4 Max).
Override with SYNAPSE_RERANKER_MODEL.
"""
from __future__ import annotations

import os
import struct
import sys
import traceback

import msgpack
from fastembed.rerank.cross_encoder import TextCrossEncoder

MODEL_ID = os.environ.get("SYNAPSE_RERANKER_MODEL", "Xenova/ms-marco-MiniLM-L-6-v2")


def _read_exact(fh, n):
    buf = bytearray()
    while len(buf) < n:
        chunk = fh.read(n - len(buf))
        if not chunk:
            return None
        buf.extend(chunk)
    return bytes(buf)


def _read_msg(fh):
    hdr = _read_exact(fh, 4)
    if hdr is None:
        return None
    (n,) = struct.unpack(">I", hdr)
    body = _read_exact(fh, n)
    if body is None:
        return None
    return msgpack.unpackb(body, raw=False)


def _write_msg(fh, obj):
    body = msgpack.packb(obj, use_bin_type=True)
    fh.write(struct.pack(">I", len(body)))
    fh.write(body)
    fh.flush()


def main() -> int:
    stdin = sys.stdin.buffer
    stdout = sys.stdout.buffer
    stderr = sys.stderr

    reranker = TextCrossEncoder(model_name=MODEL_ID)
    _write_msg(stdout, {"ready": True, "model": MODEL_ID})

    while True:
        msg = _read_msg(stdin)
        if msg is None:
            return 0
        try:
            query = msg.get("query") or ""
            docs = msg.get("docs") or []
            if not query or not docs:
                _write_msg(stdout, {"scores": []})
                continue
            scores = [float(s) for s in reranker.rerank(query, docs)]
            _write_msg(stdout, {"scores": scores})
        except Exception as e:  # noqa: BLE001
            traceback.print_exc(file=stderr)
            _write_msg(stdout, {"error": f"{type(e).__name__}: {e}"})


if __name__ == "__main__":
    sys.exit(main())
