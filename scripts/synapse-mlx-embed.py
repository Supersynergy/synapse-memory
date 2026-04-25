#!/usr/bin/env python3
"""Synapse MLX Metal embedder sidecar.

Protocol (length-prefixed msgpack over stdio):

  request:  {"texts": ["...", "..."]}
  response: {"vecs": [[f32...], ...], "dim": 384}  on success
            {"error": "msg"}                         on failure

Each message is preceded by a 4-byte big-endian length header.

Spawned by Rust `MlxMetalEmbedder` (feature `embed-mlx`). Persistent process,
one model load amortized across many embed calls.

Model: mlx-community/bge-small-en-v1.5-bf16 (default) or env SYNAPSE_MLX_MODEL.

Mean-pool + L2 normalize is applied here to match sentence-transformers /
fastembed BGE behavior. (The mlx-embeddings library defaults to CLS pooling
which produces different vectors from the canonical BGE recipe.)
"""
from __future__ import annotations

import os
import sys
import struct
import traceback

import msgpack  # type: ignore
import numpy as np
import mlx.core as mx
from mlx_embeddings.utils import load  # type: ignore

MODEL_ID = os.environ.get("SYNAPSE_MLX_MODEL", "mlx-community/bge-small-en-v1.5-bf16")


def _read_msg(fh) -> dict | None:
    hdr = fh.read(4)
    if not hdr or len(hdr) < 4:
        return None
    (n,) = struct.unpack(">I", hdr)
    body = fh.read(n)
    if len(body) < n:
        return None
    return msgpack.unpackb(body, raw=False)


def _write_msg(fh, obj: dict) -> None:
    body = msgpack.packb(obj, use_bin_type=True)
    fh.write(struct.pack(">I", len(body)))
    fh.write(body)
    fh.flush()


def _mean_pool_normalize(last_hidden, attention_mask):
    """Mean-pool with attention mask, then L2 normalize. Matches BGE canonical."""
    # last_hidden: [B, T, D]   attention_mask: [B, T]
    mask = attention_mask[..., None].astype(last_hidden.dtype)
    summed = (last_hidden * mask).sum(axis=1)
    counts = mx.maximum(mask.sum(axis=1), mx.array(1e-9, dtype=last_hidden.dtype))
    pooled = summed / counts
    norms = mx.maximum(mx.linalg.norm(pooled, axis=1, keepdims=True), mx.array(1e-12))
    return pooled / norms


def main() -> int:
    # Use raw binary stdio
    stdin = sys.stdin.buffer
    stdout = sys.stdout.buffer
    stderr = sys.stderr

    model, tokenizer = load(MODEL_ID)

    # Send ready handshake
    _write_msg(stdout, {"ready": True, "model": MODEL_ID, "dim": 384})

    while True:
        msg = _read_msg(stdin)
        if msg is None:
            return 0
        try:
            texts = msg.get("texts") or []
            if not texts:
                _write_msg(stdout, {"vecs": [], "dim": 384})
                continue

            enc = tokenizer.batch_encode_plus(
                texts,
                return_tensors="mlx",
                padding=True,
                truncation=True,
                max_length=512,
            )
            ids = enc["input_ids"]
            mask = enc["attention_mask"]
            out = model(ids, attention_mask=mask)
            # `out.last_hidden_state` is the canonical field; fall back gracefully.
            lhs = getattr(out, "last_hidden_state", None)
            if lhs is None:
                lhs = out[0] if isinstance(out, (tuple, list)) else out
            pooled = _mean_pool_normalize(lhs, mask)
            mx.eval(pooled)
            arr = np.array(pooled.tolist(), dtype=np.float32)
            vecs = [arr[i].tolist() for i in range(arr.shape[0])]
            _write_msg(stdout, {"vecs": vecs, "dim": int(arr.shape[1])})
        except Exception as e:  # noqa: BLE001
            traceback.print_exc(file=stderr)
            _write_msg(stdout, {"error": f"{type(e).__name__}: {e}"})

    return 0


if __name__ == "__main__":
    sys.exit(main())
