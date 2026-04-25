#!/usr/bin/env python3
"""Self-convert BAAI/bge-small-en-v1.5 fp32 weights to bf16 for MLX Metal.

Why this exists: upstream `mlx-community/bge-small-en-v1.5-bf16` ships weights
that produce only ~0.91 cosine similarity vs the canonical fastembed CPU
output. Converting ourselves with a clean fp32 -> bf16 cast (via MLX, no
intermediate fp16 round-trip) restores parity to >=0.99.

Inputs:  /tmp/bge-fp32/  (HF snapshot of BAAI/bge-small-en-v1.5)
Outputs: /Users/master/projects/synapse/models/bge-small-mlx-bf16/

Layout matches mlx-embeddings expectations (BertModel keys, no position_ids).
"""
from __future__ import annotations

import json
import shutil
from pathlib import Path

import mlx.core as mx
from safetensors import safe_open

SRC = Path("/tmp/bge-fp32")
DST = Path("/Users/master/projects/synapse/models/bge-small-mlx-bf16")

COPY_FILES = [
    "config.json",
    "tokenizer.json",
    "tokenizer_config.json",
    "special_tokens_map.json",
    "vocab.txt",
    "sentence_bert_config.json",
    "config_sentence_transformers.json",
    "modules.json",
]

DROP_KEYS = {"embeddings.position_ids"}


def main() -> int:
    assert SRC.exists(), f"missing source dir {SRC}"
    DST.mkdir(parents=True, exist_ok=True)

    for fname in COPY_FILES:
        sp = SRC / fname
        if sp.exists():
            shutil.copy2(sp, DST / fname)

    src_st = SRC / "model.safetensors"
    out: dict = {}
    with safe_open(src_st, framework="numpy") as f:
        keys = [k for k in f.keys() if k not in DROP_KEYS]
        for k in keys:
            t_np = f.get_tensor(k)  # fp32 numpy
            t_bf16 = mx.array(t_np).astype(mx.bfloat16)
            mx.eval(t_bf16)
            out[k] = t_bf16

    out_path = DST / "model.safetensors"
    mx.save_safetensors(str(out_path), out)
    print(f"[ok] wrote {out_path}  ({len(out)} tensors)")

    cfg_path = DST / "config.json"
    cfg = json.loads(cfg_path.read_text())
    cfg.setdefault("model_type", "bert")
    cfg.setdefault("architectures", ["BertModel"])
    cfg["torch_dtype"] = "bfloat16"
    cfg_path.write_text(json.dumps(cfg, indent=2))
    print(f"[ok] patched {cfg_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
