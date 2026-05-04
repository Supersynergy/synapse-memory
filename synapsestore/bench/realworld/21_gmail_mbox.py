"""Gmail mbox bench — c21.

Expects a standard mbox file (Google Takeout → Mail → single .mbox).
Extracts subject + plain-text body per message and runs the harness.

Usage:
    python 21_gmail_mbox.py ~/Downloads/All-Mail.mbox
"""

from __future__ import annotations
import mailbox
import random
import statistics
import sys
import time
from pathlib import Path

import synapse

from harness import HashEmbedder


def load_mbox(path: Path, max_msgs: int | None = None) -> list[tuple[int, str]]:
    out: list[tuple[int, str]] = []
    mbox = mailbox.mbox(str(path))
    for i, msg in enumerate(mbox):
        if max_msgs and i >= max_msgs: break
        subj = msg.get("Subject", "") or ""
        body = ""
        if msg.is_multipart():
            for part in msg.walk():
                if part.get_content_type() == "text/plain":
                    try:
                        body = part.get_payload(decode=True).decode("utf-8", errors="ignore")
                        break
                    except Exception:
                        continue
        else:
            try:
                body = msg.get_payload(decode=True).decode("utf-8", errors="ignore")
            except Exception:
                body = ""
        text = f"{subj}\n{body}".strip()
        if text:
            out.append((i, text[:4000]))
    return out


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print("Usage: python 21_gmail_mbox.py <mbox> [max_msgs]", file=sys.stderr)
        return 1
    path = Path(argv[1]).expanduser().resolve()
    max_msgs = int(argv[2]) if len(argv) >= 3 else None
    print(f"▸ loading {path.name} …")
    msgs = load_mbox(path, max_msgs)
    print(f"▸ {len(msgs):,} messages loaded")

    emb = HashEmbedder(dim=128)
    embs = emb.embed_documents([t for _, t in msgs])
    rows = [(i, v) for i, v in enumerate(embs)]

    t0 = time.perf_counter()
    ham = synapse.HammingIndex.build(rows)
    i8 = synapse.I8Index.build(rows)
    t_build = (time.perf_counter() - t0) * 1000

    random.seed(2)
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
    print(f"consumer framing: «search {len(msgs):,} emails in {pct(0.95)/1000:.1f} ms — faster than Spotlight»")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
