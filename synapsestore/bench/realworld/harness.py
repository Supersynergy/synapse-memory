"""Generic real-world bench harness.

Loads a corpus (dir of text files, one per doc), embeds with a user-supplied
embedder, builds Synapse indices, runs a set of queries, and prints the
Synapse "Consumer Metrics" table.

Swap the embedder for any real model; the default is a tiny deterministic
hash embedder so the harness runs without network or GPU.

Usage:
    python harness.py --src ~/notes --queries queries.txt --dim 128
"""

from __future__ import annotations
import argparse
import math
import os
import random
import statistics
import sys
import time
from pathlib import Path
from typing import Callable, Iterable, List, Sequence, Tuple

try:
    import synapse  # noqa: F401
    _SYNAPSE_AVAILABLE = True
except ImportError:  # pragma: no cover
    synapse = None  # type: ignore[assignment]
    _SYNAPSE_AVAILABLE = False


# ---------------- Embedders -----------------------------------------------

class HashEmbedder:
    """Deterministic tiny embedder — no network, no GPU, for harness demos."""
    def __init__(self, dim: int = 128): self.dim = dim
    def _h(self, s: str) -> List[float]:
        h = hash(s) & 0xFFFFFFFFFFFFFFFF
        return [math.sin((h >> (i % 32)) * 0.0001 + i) for i in range(self.dim)]
    def embed_query(self, t: str) -> List[float]: return self._h(t)
    def embed_documents(self, texts: Sequence[str]) -> List[List[float]]:
        return [self._h(t) for t in texts]


# ---------------- Harness core --------------------------------------------

TEXT_SUFFIXES = {".md", ".txt", ".rst", ".markdown", ".org"}
JSON_SUFFIXES = {".json"}
JSONL_SUFFIXES = {".jsonl", ".ndjson"}
CSV_SUFFIXES = {".csv", ".tsv"}
SQLITE_SUFFIXES = {".db", ".sqlite", ".sqlite3"}
SYNAPSE_SUFFIXES = {".syn", ".synx", ".synapse", ".brainpack"}

def _load_text(p: Path) -> list[str]:
    return [p.read_text(encoding="utf-8", errors="ignore")]

def _load_json(p: Path) -> list[str]:
    import json as _j
    try:
        data = _j.loads(p.read_text(encoding="utf-8", errors="ignore"))
    except Exception:
        return []
    if isinstance(data, list):
        return [
            str(d.get("text") or d.get("content") or d.get("body") or _j.dumps(d))
            for d in data if isinstance(d, dict)
        ]
    if isinstance(data, dict):
        return [str(data.get("text") or data.get("content") or _j.dumps(data))]
    return [str(data)]

def _load_jsonl(p: Path) -> list[str]:
    import json as _j
    out = []
    for line in p.read_text(encoding="utf-8", errors="ignore").splitlines():
        line = line.strip()
        if not line: continue
        try:
            d = _j.loads(line)
        except Exception:
            out.append(line); continue
        if isinstance(d, dict):
            out.append(str(d.get("text") or d.get("content") or d.get("body") or line))
        else:
            out.append(str(d))
    return out

def _load_csv(p: Path) -> list[str]:
    import csv
    out = []
    delim = "\t" if p.suffix.lower() == ".tsv" else ","
    with p.open(encoding="utf-8", errors="ignore") as f:
        for row in csv.reader(f, delimiter=delim):
            if row:
                out.append(" ".join(c.strip() for c in row if c))
    return out

def _load_sqlite(p: Path) -> list[str]:
    """Probe common text columns — synapse-native `.db` exports 'docs.text'."""
    import sqlite3
    try:
        con = sqlite3.connect(f"file:{p}?mode=ro", uri=True)
    except sqlite3.Error:
        return []
    out = []
    try:
        # Synapse-native Store: docs.text
        for candidate in [
            "SELECT text FROM docs WHERE text IS NOT NULL LIMIT ?",
            "SELECT content FROM documents LIMIT ?",
            "SELECT body FROM notes LIMIT ?",
        ]:
            try:
                cur = con.execute(candidate, (100_000,))
                out = [str(r[0]) for r in cur if r[0]]
                if out:
                    break
            except sqlite3.Error:
                continue
    finally:
        con.close()
    return out

def _load_synapse(p: Path) -> list[str]:
    """`.synx / .brainpack / .syn / .synapse` — native Synapse formats.

    Preferred path: `synapse.brainpack_unpack` (Rust backend) for `.brainpack`
    archives; then extract text via utf-8 scan on the resulting `.synx` body
    until the synx-native text reader is wired (tracked for synapse-py v0.3).
    """
    import re
    import tempfile
    raw: bytes | None = None
    s = p.suffix.lower()
    if s == ".brainpack" and _SYNAPSE_AVAILABLE:
        try:
            with tempfile.NamedTemporaryFile(suffix=".synx", delete=False) as tmp:
                tmp_path = tmp.name
            synapse.brainpack_unpack(str(p), tmp_path)  # type: ignore[union-attr]
            raw = Path(tmp_path).read_bytes()
            Path(tmp_path).unlink(missing_ok=True)
        except Exception:
            raw = None
    if raw is None:
        try:
            raw = p.read_bytes()
        except Exception:
            return []
        # zstd frame? strip it.
        if raw[:4] == b"\x28\xb5\x2f\xfd":
            try:
                import zstandard as zstd  # type: ignore[import-untyped]
                raw = zstd.ZstdDecompressor().decompress(raw)
            except Exception:
                pass
    # UTF-8 runs ≥ 8 chars
    return [
        s for s in (m.decode("utf-8", errors="ignore") for m in re.findall(rb"[\x20-\x7e]{8,}", raw))
        if s.strip()
    ]

def _ext_loader(p: Path):
    s = p.suffix.lower()
    if s in TEXT_SUFFIXES:    return _load_text
    if s in JSON_SUFFIXES:    return _load_json
    if s in JSONL_SUFFIXES:   return _load_jsonl
    if s in CSV_SUFFIXES:     return _load_csv
    if s in SQLITE_SUFFIXES:  return _load_sqlite
    if s in SYNAPSE_SUFFIXES: return _load_synapse
    return None


def load_corpus(src: Path, max_docs: int | None = None) -> List[Tuple[str, str]]:
    """Return list of (doc_id, text) from a file OR a dir of files.

    Supported extensions: .md/.txt/.rst/.markdown/.org (plain text),
    .json/.jsonl/.ndjson, .csv/.tsv, .db/.sqlite, .synx/.brainpack/.syn/.synapse.

    When `src` is a single file, that one file is loaded (possibly returning
    many docs).  When it's a dir, we walk recursively.
    """
    out: List[Tuple[str, str]] = []
    paths: list[Path] = [src] if src.is_file() else [p for p in sorted(src.rglob("*")) if p.is_file()]
    for p in paths:
        loader = _ext_loader(p)
        if loader is None: continue
        try:
            for i, text in enumerate(loader(p)):
                if not text: continue
                key = str(p.relative_to(src)) if src.is_dir() else p.name
                out.append((f"{key}#{i}", text))
                if max_docs and len(out) >= max_docs: return out
        except Exception:
            continue
    return out


def build_indices(rows: List[Tuple[int, List[float]]]):
    if not _SYNAPSE_AVAILABLE:
        raise RuntimeError("synapse not installed; run: maturin develop --release --features simsimd")
    ham = synapse.HammingIndex.build(rows)
    i8 = synapse.I8Index.build(rows)
    return ham, i8


def run_queries(
    embedder,
    ham, i8,
    queries: Sequence[str],
    k: int = 10,
    candidates: int = 80,
) -> List[float]:
    durations_us: List[float] = []
    for q in queries:
        vec = embedder.embed_query(q)
        t0 = time.perf_counter()
        if ham and i8 and i8.len() >= 1_000:
            _ = synapse.rerank(ham, i8, vec, k=k, candidates=candidates)
        else:
            _ = i8.search(vec, k=k)
        durations_us.append((time.perf_counter() - t0) * 1e6)
    return durations_us


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="Synapse real-world bench harness")
    ap.add_argument("--src", type=Path, required=True, help="corpus dir (markdown/txt files)")
    ap.add_argument("--queries", type=Path, help="newline-separated queries file")
    ap.add_argument("--dim", type=int, default=128)
    ap.add_argument("--max-docs", type=int, default=None)
    ap.add_argument("--k", type=int, default=10)
    ap.add_argument("--candidates", type=int, default=80)
    args = ap.parse_args(argv)

    embedder = HashEmbedder(dim=args.dim)

    print(f"▸ loading corpus from {args.src} …")
    docs = load_corpus(args.src, args.max_docs)
    if not docs:
        print("no documents found", file=sys.stderr); return 1
    print(f"▸ loaded {len(docs):,} docs")

    t_embed = time.perf_counter()
    embs = embedder.embed_documents([t for _, t in docs])
    t_embed = time.perf_counter() - t_embed
    rows = [(i, v) for i, v in enumerate(embs)]

    t_build = time.perf_counter()
    ham, i8 = build_indices(rows)
    t_build = time.perf_counter() - t_build

    # Queries fallback: reuse 20 random doc-titles as queries.
    if args.queries and args.queries.exists():
        queries = [q.strip() for q in args.queries.read_text().splitlines() if q.strip()]
    else:
        random.seed(0)
        queries = [docs[random.randrange(len(docs))][0] for _ in range(20)]

    durs = run_queries(embedder, ham, i8, queries, k=args.k, candidates=args.candidates)
    durs_sorted = sorted(durs)
    def pct(p: float) -> float: return durs_sorted[min(len(durs_sorted) - 1, int(p * len(durs_sorted)))]

    print()
    print(f"▸ corpus embed time     {t_embed:>7.2f} s")
    print(f"▸ index build time      {t_build * 1000:>7.1f} ms")
    print(f"▸ queries run           {len(durs)}")
    print(f"▸ p50 latency           {statistics.median(durs):>7.0f} µs")
    print(f"▸ p95 latency           {pct(0.95):>7.0f} µs")
    print(f"▸ p99 latency           {pct(0.99):>7.0f} µs")
    print(f"▸ mean QPS              {1e6 / max(statistics.mean(durs), 1e-3):>7.0f}")
    print()
    print(f"consumer framing: «find anything across {len(docs):,} docs in {pct(0.95) / 1000:.1f} ms (95 %)»")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
