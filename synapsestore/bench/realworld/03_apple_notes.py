"""Apple Notes SQLite bench — a03.

Reads the NoteStore.sqlite that lives at:
    ~/Library/Group Containers/group.com.apple.notes/NoteStore.sqlite

Notes are stored as gzipped protobuf — this harness uses the plain-text
fallback in `ZICCLOUDSYNCINGOBJECT.ZSNIPPET` + `ZTITLE1` which preserves the
searchable subset. That's plenty for a realistic bench.

Usage:
    # copy the db out first so Notes.app isn't locked
    cp ~/Library/Group\\ Containers/group.com.apple.notes/NoteStore.sqlite /tmp/notes.db
    python 03_apple_notes.py /tmp/notes.db
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
SELECT Z_PK AS id, IFNULL(ZTITLE1, '') || ' ' || IFNULL(ZSNIPPET, '') AS text
FROM   ZICCLOUDSYNCINGOBJECT
WHERE  ZSNIPPET IS NOT NULL OR ZTITLE1 IS NOT NULL
"""


def load_notes(db: Path) -> list[tuple[int, str]]:
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    try:
        cur = con.execute(SQL)
        return [(int(rowid), text.strip()) for rowid, text in cur if text and text.strip()]
    finally:
        con.close()


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print("Usage: python 03_apple_notes.py <path/to/NoteStore.sqlite>", file=sys.stderr)
        return 1
    db = Path(argv[1]).expanduser().resolve()
    if not db.exists():
        print(f"not found: {db}", file=sys.stderr); return 1
    notes = load_notes(db)
    print(f"▸ {len(notes):,} notes loaded")
    if not notes:
        return 1

    emb = HashEmbedder(dim=128)
    rows = [(nid, emb.embed_query(txt)) for nid, txt in notes]
    t0 = time.perf_counter()
    ham = synapse.HammingIndex.build(rows)
    i8 = synapse.I8Index.build(rows)
    t_build = (time.perf_counter() - t0) * 1000

    random.seed(4)
    queries = [notes[random.randrange(len(notes))][1][:120] for _ in range(20)]
    durs = []
    for q in queries:
        v = emb.embed_query(q)
        t = time.perf_counter()
        _ = synapse.rerank(ham, i8, v, k=10, candidates=80) if len(notes) >= 1000 else i8.search(v, k=10)
        durs.append((time.perf_counter() - t) * 1e6)
    ds = sorted(durs)
    def pct(p): return ds[min(len(ds) - 1, int(p * len(ds)))]

    print(f"▸ build indices          {t_build:>7.1f} ms")
    print(f"▸ p50 / p95 / p99        {statistics.median(durs):>5.0f} / {pct(0.95):>5.0f} / {pct(0.99):>5.0f} µs")
    print(f"consumer framing: «every Apple Note, searchable in {pct(0.95)/1000:.1f} ms»")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
