"""ChatGPT history bench — b11.

Expects a `conversations.json` export from chat.openai.com (Data Controls →
Export data). Unpacks the nested `mapping` structure to a flat (id, text)
list, then runs the generic harness.

Usage:
    python 11_chatgpt_history.py ~/Downloads/conversations.json
"""

from __future__ import annotations
import json
import math
import random
import statistics
import sys
import time
from pathlib import Path

import synapse

from harness import HashEmbedder


def flatten(conversations_json: list) -> list[tuple[int, str]]:
    """Return (turn_id, text) across every conversation message node."""
    out: list[tuple[int, str]] = []
    turn = 0
    for conv in conversations_json:
        mapping = conv.get("mapping", {})
        for _, node in mapping.items():
            msg = node.get("message")
            if not msg: continue
            content = msg.get("content", {})
            parts = content.get("parts")
            if not parts: continue
            text = " ".join(p for p in parts if isinstance(p, str)).strip()
            if text:
                out.append((turn, text))
                turn += 1
    return out


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print("Usage: python 11_chatgpt_history.py <conversations.json>", file=sys.stderr)
        return 1
    path = Path(argv[1]).expanduser().resolve()
    conversations = json.loads(path.read_text())
    msgs = flatten(conversations)
    if not msgs:
        print("no messages parsed", file=sys.stderr); return 1
    print(f"▸ {len(msgs):,} messages flattened from {path.name}")

    emb = HashEmbedder(dim=128)
    embs = emb.embed_documents([t for _, t in msgs])
    rows = [(i, v) for i, v in enumerate(embs)]

    t0 = time.perf_counter()
    ham = synapse.HammingIndex.build(rows)
    i8 = synapse.I8Index.build(rows)
    t_build = (time.perf_counter() - t0) * 1000

    # 20 random past messages as queries — simulates "what did I say about X"
    random.seed(1)
    queries = [msgs[random.randrange(len(msgs))][1][:120] for _ in range(20)]

    durs = []
    for q in queries:
        vec = emb.embed_query(q)
        t = time.perf_counter()
        _ = synapse.rerank(ham, i8, vec, k=10, candidates=80) if len(msgs) >= 1000 else i8.search(vec, k=10)
        durs.append((time.perf_counter() - t) * 1e6)

    ds = sorted(durs)
    def pct(p: float) -> float: return ds[min(len(ds) - 1, int(p * len(ds)))]

    print(f"▸ build indices          {t_build:>7.1f} ms")
    print(f"▸ p50                    {statistics.median(durs):>7.0f} µs")
    print(f"▸ p95                    {pct(0.95):>7.0f} µs")
    print(f"▸ p99                    {pct(0.99):>7.0f} µs")
    print()
    print(f"consumer framing: «every ChatGPT you ever had, recalled in {pct(0.95)/1000:.1f} ms»")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
