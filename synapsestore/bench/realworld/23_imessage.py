"""iMessage SQLite bench — a23.

Reads the macOS iMessage database that lives at:
    ~/Library/Messages/chat.db

Messages are extracted from the `message` table, filtered for plain text
messages with content. This provides a realistic bench of conversational data.

Usage:
    python 23_imessage.py
    # or with explicit path:
    python 23_imessage.py /path/to/chat.db
"""

from __future__ import annotations
import random
import sqlite3
import statistics
import sys
import time
from pathlib import Path

import synapse

from harness import HashEmbedder


SQL = """
SELECT ROWID as id, text
FROM   message
WHERE  text IS NOT NULL AND length(text) > 0
"""


def load_messages(db: Path) -> list[tuple[int, str]]:
    """Load iMessage text from read-only SQLite database."""
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    try:
        cur = con.execute(SQL)
        return [
            (int(rowid), text.strip())
            for rowid, text in cur
            if text and text.strip() and len(text.strip()) > 3
        ]
    finally:
        con.close()


def main(argv: list[str]) -> int:
    # Default to ~/Library/Messages/chat.db if no argument provided
    if len(argv) < 2:
        db = Path("~/Library/Messages/chat.db").expanduser().resolve()
    else:
        db = Path(argv[1]).expanduser().resolve()

    if not db.exists():
        print(f"not found: {db}", file=sys.stderr)
        return 1

    messages = load_messages(db)
    print(f"▸ {len(messages):,} iMessages loaded")
    if not messages:
        return 1

    emb = HashEmbedder(dim=128)
    rows = [(mid, emb.embed_query(txt)) for mid, txt in messages]
    t0 = time.perf_counter()
    ham = synapse.HammingIndex.build(rows)
    i8 = synapse.I8Index.build(rows)
    t_build = (time.perf_counter() - t0) * 1000

    random.seed(6)
    queries = [messages[random.randrange(len(messages))][1][:120] for _ in range(20)]
    durs = []
    for q in queries:
        v = emb.embed_query(q)
        t = time.perf_counter()
        _ = (
            synapse.rerank(ham, i8, v, k=10, candidates=80)
            if len(messages) >= 1000
            else i8.search(v, k=10)
        )
        durs.append((time.perf_counter() - t) * 1e6)

    ds = sorted(durs)

    def pct(p: float) -> float:
        return ds[min(len(ds) - 1, int(p * len(ds)))]

    print(f"▸ build indices          {t_build:>7.1f} ms")
    print(
        f"▸ p50 / p95 / p99        {statistics.median(durs):>5.0f} / {pct(0.95):>5.0f} / {pct(0.99):>5.0f} µs"
    )
    print(f"consumer framing: «{len(messages):,} iMessages searched in {pct(0.95)/1000:.1f} ms»")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
