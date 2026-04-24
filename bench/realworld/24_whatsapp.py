"""WhatsApp chat export bench — c24.

Expects a WhatsApp chat text export (one message per line).
Supports two common formats:
  - DD.MM.YY, HH:MM - Name: message
  - [DD.MM.YY, HH:MM:SS] Name: message

Skips media lines, system messages, and lines that don't parse as messages.

Usage:
    python 24_whatsapp.py <chat.txt> [max_msgs]
"""

from __future__ import annotations
import random
import re
import statistics
import sys
import time
from pathlib import Path

import synapse

from harness import HashEmbedder


def load_whatsapp(path: Path, max_msgs: int | None = None) -> list[tuple[int, str]]:
    out: list[tuple[int, str]] = []
    try:
        lines = path.read_text(encoding="utf-8", errors="ignore").splitlines()
    except Exception:
        return out

    # Regex patterns for WhatsApp message formats
    # Format 1: DD.MM.YY, HH:MM - Name: message
    pattern1 = re.compile(r"^\d{1,2}\.\d{1,2}\.\d{2,4},\s+\d{1,2}:\d{2}\s*-\s*(.+?):\s*(.+)$")
    # Format 2: [DD.MM.YY, HH:MM:SS] Name: message
    pattern2 = re.compile(r"^\[\d{1,2}\.\d{1,2}\.\d{2,4},\s+\d{1,2}:\d{2}:\d{2}\]\s*(.+?):\s*(.+)$")

    for line in lines:
        line = line.strip()
        if not line:
            continue

        # Skip media attachments and system messages
        if "<Media omitted>" in line or "Messages and calls are encrypted" in line:
            continue
        if line.startswith("This message was deleted"):
            continue

        # Try format 1
        m1 = pattern1.match(line)
        if m1:
            text = m1.group(2).strip()
            if text:
                out.append((len(out), text[:2000]))
                if max_msgs and len(out) >= max_msgs:
                    return out
            continue

        # Try format 2
        m2 = pattern2.match(line)
        if m2:
            text = m2.group(2).strip()
            if text:
                out.append((len(out), text[:2000]))
                if max_msgs and len(out) >= max_msgs:
                    return out
            continue

    return out


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print("Usage: python 24_whatsapp.py <chat.txt> [max_msgs]", file=sys.stderr)
        return 1
    path = Path(argv[1]).expanduser().resolve()
    max_msgs = int(argv[2]) if len(argv) >= 3 else None
    print(f"▸ loading {path.name} …")
    msgs = load_whatsapp(path, max_msgs)
    print(f"▸ {len(msgs):,} WhatsApp messages loaded")
    if not msgs:
        return 1

    emb = HashEmbedder(dim=128)
    rows = [(i, emb.embed_query(t)) for i, t in msgs]
    t0 = time.perf_counter()
    ham = synapse.HammingIndex.build(rows)
    i8 = synapse.I8Index.build(rows)
    t_build = (time.perf_counter() - t0) * 1000

    random.seed(5)
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
    print(f"consumer framing: «{len(msgs):,} WhatsApp messages searched in {pct(0.95)/1000:.1f} ms»")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
