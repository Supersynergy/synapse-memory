"""Slack-workspace export bench — c22.

Expects a Slack admin export (workspace settings → Import/Export → Export).
Walks every channel dir, loads JSON-per-day files, flattens `text` fields.

Usage:
    python 22_slack_export.py ~/Downloads/SlackExport
"""

from __future__ import annotations
import json
import random
import statistics
import sys
import time
from pathlib import Path

import synapse

from harness import HashEmbedder


def load_slack(export_dir: Path, max_msgs: int | None = None) -> list[tuple[int, str]]:
    out: list[tuple[int, str]] = []
    for channel_dir in export_dir.iterdir():
        if not channel_dir.is_dir():
            continue
        for day in sorted(channel_dir.glob("*.json")):
            try:
                data = json.loads(day.read_text(encoding="utf-8", errors="ignore"))
            except Exception:
                continue
            for msg in data:
                t = msg.get("text", "").strip()
                if t:
                    out.append((len(out), t[:2000]))
                    if max_msgs and len(out) >= max_msgs:
                        return out
    return out


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print("Usage: python 22_slack_export.py <export_dir> [max_msgs]", file=sys.stderr)
        return 1
    root = Path(argv[1]).expanduser().resolve()
    max_msgs = int(argv[2]) if len(argv) >= 3 else None
    print(f"▸ scanning {root} …")
    msgs = load_slack(root, max_msgs)
    print(f"▸ {len(msgs):,} slack messages loaded")
    if not msgs:
        return 1

    emb = HashEmbedder(dim=128)
    rows = [(i, emb.embed_query(t)) for i, t in msgs]
    t0 = time.perf_counter()
    ham = synapse.HammingIndex.build(rows)
    i8 = synapse.I8Index.build(rows)
    t_build = (time.perf_counter() - t0) * 1000

    random.seed(3)
    queries = [msgs[random.randrange(len(msgs))][1][:120] for _ in range(20)]
    durs = []
    for q in queries:
        v = emb.embed_query(q)
        t = time.perf_counter()
        _ = synapse.rerank(ham, i8, v, k=10, candidates=80) if len(msgs) >= 1000 else i8.search(v, k=10)
        durs.append((time.perf_counter() - t) * 1e6)
    ds = sorted(durs)
    def pct(p): return ds[min(len(ds) - 1, int(p * len(ds)))]

    print(f"▸ build indices          {t_build:>7.1f} ms")
    print(f"▸ p50 / p95 / p99        {statistics.median(durs):>5.0f} / {pct(0.95):>5.0f} / {pct(0.99):>5.0f} µs")
    print(f"consumer framing: «{len(msgs):,} slack messages, searched in {pct(0.95)/1000:.1f} ms»")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
