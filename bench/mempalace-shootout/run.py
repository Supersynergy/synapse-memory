#!/usr/bin/env python3
"""
MemPalace shootout: ChromaDB vs Synapse backend.

Metrics captured:
  - insert ops/s
  - query latency p50 / p99 (ms)
  - R@5, R@10 (held-out recall)
  - RSS MB, disk MB, wall time (s)

Modes:
  --data PATH          explicit dataset path (default: lme_s_50.json)
  --full               load lme_s_500.json (must be present)
  --split 40/10        held-out split: train 40, test 10 (default for -50)
  --backend chroma|synapse|both

Held-out eval (honest R@K):
  Train split: index conversation_str. Test split: query using question text.
  R@K = fraction of test questions where the correct question_id appears in top-K
  results. This matches MemPalace's published 96.6% R@5 baseline.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import re
import resource
import shutil
import socket
import struct
import tempfile
import time
from typing import Any

HERE = pathlib.Path(__file__).parent
SYNAPSE_SOCK = "/tmp/synapse.sock"
DATA_50 = HERE.parent / "longmemeval" / "data" / "lme_s_50.json"
DATA_500 = HERE.parent / "longmemeval" / "data" / "lme_s_500.json"


# ---------------------------------------------------------------------------
# Embedder
# ---------------------------------------------------------------------------

def _get_embedder():
    try:
        from sentence_transformers import SentenceTransformer
        return SentenceTransformer("sentence-transformers/all-MiniLM-L6-v2")
    except ImportError:
        raise SystemExit(
            "sentence-transformers not installed.\n"
            "Run: uv pip install -p ~/.venvs/mempalace-bench sentence-transformers"
        )


# ---------------------------------------------------------------------------
# Daemon embed bridge (Track A)
# ---------------------------------------------------------------------------

def _daemon_embed(text: str, sock_path: str = SYNAPSE_SOCK) -> list[float] | None:
    """Send Request::Embed{text} over synapsed socket, return vec or None."""
    try:
        import msgpack
        req = msgpack.packb({"op": "Embed", "args": {"text": text}}, use_bin_type=True)
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
            s.settimeout(5.0)
            s.connect(sock_path)
            s.sendall(struct.pack("<I", len(req)) + req)
            lenbuf = s.recv(4)
            if len(lenbuf) < 4:
                return None
            n = struct.unpack("<I", lenbuf)[0]
            data = b""
            while len(data) < n:
                chunk = s.recv(n - len(data))
                if not chunk:
                    break
                data += chunk
            resp = msgpack.unpackb(data, raw=False)
            if isinstance(resp, dict) and "Embed" in resp:
                return resp["Embed"]["vec"]
            return None
    except Exception:
        return None


def _daemon_rerank(query: str, candidates: list[dict], top_k: int, sock_path: str = SYNAPSE_SOCK) -> list[dict] | None:
    """Send Request::Rerank over synapsed socket. candidates = list of Hit dicts."""
    try:
        import msgpack
        req = msgpack.packb(
            {"op": "Rerank", "args": {"query": query, "candidates": candidates, "top_k": top_k}},
            use_bin_type=True,
        )
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
            s.settimeout(10.0)
            s.connect(sock_path)
            s.sendall(struct.pack("<I", len(req)) + req)
            lenbuf = s.recv(4)
            if len(lenbuf) < 4:
                return None
            n = struct.unpack("<I", lenbuf)[0]
            data = b""
            while len(data) < n:
                chunk = s.recv(n - len(data))
                if not chunk:
                    break
                data += chunk
            resp = msgpack.unpackb(data, raw=False)
            if isinstance(resp, dict) and "Hits" in resp:
                return resp["Hits"]
            return None
    except Exception:
        return None


def _daemon_alive(sock_path: str = SYNAPSE_SOCK) -> bool:
    try:
        import msgpack
        req = msgpack.packb({"op": "Ping", "args": None}, use_bin_type=True)
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
            s.settimeout(2.0)
            s.connect(sock_path)
            s.sendall(struct.pack("<I", len(req)) + req)
            lenbuf = s.recv(4)
            return len(lenbuf) == 4
    except Exception:
        return False


# ---------------------------------------------------------------------------
# Session chunker (mirrors chunk_session in mcp.rs)
# ---------------------------------------------------------------------------

_SESSION_RE = re.compile(r"Session Timestamp:[^\n]*\n", re.IGNORECASE)
_MSG_RE = re.compile(r'\{"role"\s*:\s*"([^"]+)"\s*,\s*"content"\s*:\s*"((?:[^"\\]|\\.)*)"\}', re.DOTALL)

_WINDOW = 400
_OVERLAP = 50
_THRESHOLD = 1000


def _split_content(content: str, prefix: str, qid: str, uid_base: str) -> list[dict]:
    content = content.strip()
    if not content:
        return []
    if len(content) <= _THRESHOLD:
        return [{"text": f"{prefix}{content}"[:600], "qid": qid, "uid": f"{uid_base}-w0"}]
    step = _WINDOW - _OVERLAP
    chunks = []
    win = 0
    start = 0
    while start < len(content):
        end = min(start + _WINDOW, len(content))
        sub = content[start:end]
        chunks.append({"text": f"{prefix}{sub}", "qid": qid, "uid": f"{uid_base}-w{win}"})
        win += 1
        if end == len(content):
            break
        start += step
    return chunks


def _chunk_conversation(conv: str, qid: str) -> list[dict]:
    """Per-message chunks: one chunk per message (400-char windows for long messages)."""
    chunks = []
    chunk_idx = 0
    session_blocks = _SESSION_RE.split(conv)
    session_headers = _SESSION_RE.findall(conv)

    for block_i, body in enumerate(session_blocks):
        session_ts = session_headers[block_i - 1].strip() if block_i > 0 and block_i - 1 < len(session_headers) else ""
        found_any = False
        for m in _MSG_RE.finditer(body):
            role = m.group(1)
            content = m.group(2).replace('\\"', '"').replace('\\n', '\n').replace('\\\\', '\\').strip()
            if not content:
                continue
            found_any = True
            prefix = f"[{role}|{session_ts}|msg{chunk_idx}] " if session_ts else f"[{role}|msg{chunk_idx}] "
            chunks.extend(_split_content(content, prefix, qid, f"{qid}-c{chunk_idx}"))
            chunk_idx += 1
        if not found_any and body.strip():
            sub = body.strip()[:_THRESHOLD]
            chunks.extend(_split_content(sub, "[session] ", qid, f"{qid}-c{chunk_idx}"))
            chunk_idx += 1
    return chunks


# ---------------------------------------------------------------------------
# Dataset helpers
# ---------------------------------------------------------------------------

def load_records(path: pathlib.Path) -> list[dict]:
    with open(path) as f:
        rows = json.load(f)
    out = []
    for i, row in enumerate(rows):
        out.append({
            "id": row.get("question_id", str(i)),
            "question": row.get("question", ""),
            "answer": row.get("answer", ""),
            "text": row.get("conversation_str", row.get("question", "")),
            "question_type": row.get("question_type", ""),
        })
    return out


def make_splits(records: list[dict], n_train: int, n_test: int) -> tuple[list[dict], list[dict]]:
    """Return (train, test) — train docs are indexed, test docs are queries."""
    total = len(records)
    if n_train + n_test > total:
        n_train = max(1, total - n_test)
    train = records[:n_train]
    test = records[n_train:n_train + n_test]
    return train, test


# ---------------------------------------------------------------------------
# Metric helpers
# ---------------------------------------------------------------------------

def rss_mb() -> float:
    usage = resource.getrusage(resource.RUSAGE_SELF)
    return usage.ru_maxrss / (1024 * 1024)


def disk_mb(path: str) -> float:
    p = pathlib.Path(path)
    if not p.exists():
        return 0.0
    if p.is_dir():
        total = sum(f.stat().st_size for f in p.rglob("*") if f.is_file())
    else:
        total = p.stat().st_size
    return total / (1024 * 1024)


def pct(lst: list[float], p: int) -> float:
    if not lst:
        return float("nan")
    lst_sorted = sorted(lst)
    idx = max(0, int(len(lst_sorted) * p / 100) - 1)
    return lst_sorted[idx]


# ---------------------------------------------------------------------------
# Benchmark runner (held-out mode)
# ---------------------------------------------------------------------------

def run_backend_heldout(
    backend_name: str,
    train: list[dict],
    test: list[dict],
    embedder,
    store_dir: str,
    use_rerank: bool = False,
) -> dict:
    """
    Held-out bench:
      - Index train records (conversation_str, tagged with question_id in metadata)
      - For each test record, query with question text, check if correct id in top-K
    """
    result = {"backend": backend_name, "error": None,
              "n_train": len(train), "n_test": len(test)}
    t0_wall = time.perf_counter()

    try:
        if backend_name == "chroma":
            import chromadb
            client = chromadb.PersistentClient(path=store_dir)
            collection = client.get_or_create_collection("lme_heldout")

            def _add(ids, embeddings, documents, metadatas):
                collection.add(ids=ids, embeddings=embeddings,
                               documents=documents, metadatas=metadatas)

            def _query(emb, n):
                return collection.query(query_embeddings=[emb], n_results=n)

        elif backend_name == "synapse":
            if _daemon_alive():
                from mempalace_synapse.backend import SynapseRpcBackend
                be = SynapseRpcBackend()
                print("    [synapse] using SynapseRpcBackend (daemon alive)")
            else:
                from mempalace_synapse.backend import SynapseBackend
                be = SynapseBackend(persist_dir=store_dir)
                print("    [synapse] using SynapseBackend (FFI, daemon not reachable)")
            collection = be.get_collection("lme_heldout")

            def _add(ids, embeddings, documents, metadatas):
                collection.add(ids=ids, embeddings=embeddings,
                               documents=documents, metadatas=metadatas)

            def _query(emb, n):
                return collection.query(query_embeddings=[emb], n_results=n)

        else:
            raise ValueError(f"Unknown backend: {backend_name}")
    except ImportError as e:
        result["error"] = f"IMPORT_ERROR: {e}"
        return result

    # --- embed train docs ---
    train_texts = [r["text"][:3000] for r in train]
    t_embed = time.perf_counter()
    train_embeddings = embedder.encode(train_texts, batch_size=32,
                                       show_progress_bar=False).tolist()
    result["embed_train_s"] = round(time.perf_counter() - t_embed, 3)

    # --- insert train ---
    insert_times = []
    BATCH = 10
    for start in range(0, len(train), BATCH):
        batch_r = train[start:start + BATCH]
        batch_e = train_embeddings[start:start + BATCH]
        ids = [r["id"] for r in batch_r]
        docs = [r["text"][:3000] for r in batch_r]
        metas = [{"question": r["question"][:200], "answer": r["answer"]} for r in batch_r]
        t = time.perf_counter()
        _add(ids=ids, embeddings=batch_e, documents=docs, metadatas=metas)
        insert_times.append(time.perf_counter() - t)

    total_insert_s = sum(insert_times)
    result["insert_ops_per_s"] = round(len(train) / max(total_insert_s, 1e-6))

    # --- embed test queries ---
    test_questions = [r["question"] for r in test]
    test_embeddings = embedder.encode(test_questions, batch_size=32,
                                      show_progress_bar=False).tolist()

    # --- query (held-out) ---
    K_VALUES = [5, 10]
    hits_at = {k: 0 for k in K_VALUES}
    query_latencies = []

    for r, qemb in zip(test, test_embeddings):
        t = time.perf_counter()
        # fetch top-50 for rerank, or max(K_VALUES) otherwise
        fetch_n = 50 if use_rerank else max(K_VALUES)
        qr = _query(qemb, fetch_n)
        query_latencies.append((time.perf_counter() - t) * 1000)

        returned_ids = qr["ids"][0] if qr and qr.get("ids") else []
        if use_rerank and returned_ids:
            candidates = [
                {"id": int(rid) if str(rid).lstrip("-").isdigit() else 0,
                 "text": doc, "score": 0.5, "uri": None, "title": None}
                for rid, doc in zip(
                    returned_ids,
                    (qr.get("documents") or [[]])[0] or [""] * len(returned_ids),
                )
            ]
            reranked = _daemon_rerank(r["question"], candidates, max(K_VALUES))
            if reranked:
                returned_ids = [str(h.get("id", "")) for h in reranked]
        for k in K_VALUES:
            if r["id"] in returned_ids[:k]:
                hits_at[k] += 1

    n_test = len(test)
    for k in K_VALUES:
        result[f"recall_at_{k}"] = round(hits_at[k] / n_test, 4) if n_test else 0.0

    result["query_p50_ms"] = round(pct(query_latencies, 50), 3)
    result["query_p99_ms"] = round(pct(query_latencies, 99), 3)
    result["rss_mb"] = round(rss_mb(), 1)
    result["disk_mb"] = round(disk_mb(store_dir), 2)
    result["wall_s"] = round(time.perf_counter() - t0_wall, 2)
    return result


# ---------------------------------------------------------------------------
# Self-retrieval runner (old mode, kept for -50 self-match)
# ---------------------------------------------------------------------------

def run_backend_selfmatch(
    backend_name: str,
    records: list[dict],
    embedder,
    store_dir: str,
) -> dict:
    result = {"backend": backend_name, "error": None, "mode": "self-match"}
    t0_wall = time.perf_counter()

    try:
        if backend_name == "chroma":
            import chromadb
            client = chromadb.PersistentClient(path=store_dir)
            collection = client.get_or_create_collection("lme_s_bench")

            def _add(ids, embeddings, documents, metadatas):
                collection.add(ids=ids, embeddings=embeddings,
                               documents=documents, metadatas=metadatas)

            def _query(emb, n):
                return collection.query(query_embeddings=[emb], n_results=n)

        elif backend_name == "synapse":
            if _daemon_alive():
                from mempalace_synapse.backend import SynapseRpcBackend
                be = SynapseRpcBackend()
                print("    [synapse] using SynapseRpcBackend (daemon alive)")
            else:
                from mempalace_synapse.backend import SynapseBackend
                be = SynapseBackend(persist_dir=store_dir)
                print("    [synapse] using SynapseBackend (FFI, daemon not reachable)")
            collection = be.get_collection("lme_s_bench")

            def _add(ids, embeddings, documents, metadatas):
                collection.add(ids=ids, embeddings=embeddings,
                               documents=documents, metadatas=metadatas)

            def _query(emb, n):
                return collection.query(query_embeddings=[emb], n_results=n)

        else:
            raise ValueError(f"Unknown backend: {backend_name}")
    except ImportError as e:
        result["error"] = f"IMPORT_ERROR: {e}"
        return result

    texts = [r["text"][:2000] for r in records]
    embeddings = embedder.encode(texts, batch_size=32, show_progress_bar=False).tolist()
    result["embed_s"] = round(time.perf_counter() - t0_wall, 3)

    insert_times = []
    for start in range(0, len(records), 10):
        batch_r = records[start:start + 10]
        batch_e = embeddings[start:start + 10]
        ids = [r["id"] for r in batch_r]
        docs = [r["text"][:2000] for r in batch_r]
        metas = [{"answer": r["answer"], "question": r["question"][:200]} for r in batch_r]
        t = time.perf_counter()
        _add(ids=ids, embeddings=batch_e, documents=docs, metadatas=metas)
        insert_times.append(time.perf_counter() - t)

    result["insert_ops_per_s"] = round(len(records) / max(sum(insert_times), 1e-6))

    query_latencies = []
    correct_at_5 = 0
    for r, qemb in zip(records, embeddings):
        t = time.perf_counter()
        qr = _query(qemb, 5)
        query_latencies.append((time.perf_counter() - t) * 1000)
        returned_ids = qr["ids"][0] if qr and qr.get("ids") else []
        if r["id"] in returned_ids:
            correct_at_5 += 1

    result["recall_at_5"] = round(correct_at_5 / len(records), 4) if records else 0.0
    result["query_p50_ms"] = round(pct(query_latencies, 50), 3)
    result["query_p99_ms"] = round(pct(query_latencies, 99), 3)
    result["rss_mb"] = round(rss_mb(), 1)
    result["disk_mb"] = round(disk_mb(store_dir), 2)
    result["wall_s"] = round(time.perf_counter() - t0_wall, 2)
    return result


# ---------------------------------------------------------------------------
# Sweep bench — session-level chunking + hybrid query (Track C)
# ---------------------------------------------------------------------------

def run_backend_sweep(
    backend_name: str,
    records: list[dict],
    embedder,
    store_dir: str,
    use_daemon: bool = False,
    use_rerank: bool = False,
) -> dict:
    """
    Per-record sweep evaluation (correct LME setup):
    For each record: index its own per-message chunks, query with question text.
    R@K = top-K hits contain a chunk whose text includes the answer string.
    Each record gets a fresh collection (clean DB per record, no cross-contamination).
    """
    result = {"backend": backend_name, "error": None, "mode": "sweep-per-record",
              "n_records": len(records), "daemon_embed": use_daemon}
    t0_wall = time.perf_counter()

    K_VALUES = [5, 10]
    hits_at = {k: 0 for k in K_VALUES}
    query_latencies = []
    total_chunks_all = 0
    insert_times_all = []
    BATCH = 128

    for rec_i, rec in enumerate(records):
        chunks = _chunk_conversation(rec["text"], rec["id"])
        if not chunks:
            continue
        total_chunks_all += len(chunks)

        # Fresh store per record
        rec_dir = os.path.join(store_dir, f"rec_{rec_i}")
        os.makedirs(rec_dir, exist_ok=True)

        try:
            if backend_name == "chroma":
                import chromadb
                client = chromadb.PersistentClient(path=rec_dir)
                col = client.get_or_create_collection("lme-rec")

                def _add_batch(uids, embs, texts, qids):
                    col.add(ids=uids, embeddings=embs, documents=texts,
                            metadatas=[{"qid": q} for q in qids])

                def _query(emb, n):
                    n = min(n, len(chunks))
                    return col.query(query_embeddings=[emb], n_results=n)

                def _top_texts(qr, k):
                    return (qr.get("documents") or [[]])[0][:k]

            elif backend_name == "synapse":
                # sweep needs per-record isolation: always use FFI backend (own DB per rec_dir)
                from mempalace_synapse.backend import SynapseBackend
                be = SynapseBackend(persist_dir=rec_dir)
                col = be.get_collection("lme-rec")

                def _add_batch(uids, embs, texts, qids):
                    col.add(ids=uids, embeddings=embs, documents=texts,
                            metadatas=[{"qid": q} for q in qids])

                def _query(emb, n):
                    n = min(n, len(chunks))
                    return col.query(query_embeddings=[emb], n_results=n)

                def _top_texts(qr, k):
                    return (qr.get("documents") or [[]])[0][:k]

            else:
                raise ValueError(f"Unknown backend: {backend_name}")
        except ImportError as e:
            result["error"] = f"IMPORT_ERROR: {e}"
            return result

        # Batch embed all chunks for this record
        texts = [ch["text"][:500] for ch in chunks]
        embs = []
        for start in range(0, len(texts), BATCH):
            batch = texts[start:start + BATCH]
            embs.extend(embedder.encode(batch, batch_size=BATCH, show_progress_bar=False).tolist())

        # Insert
        t = time.perf_counter()
        _add_batch(
            [ch["uid"] for ch in chunks],
            embs,
            [ch["text"][:500] for ch in chunks],
            [ch["qid"] for ch in chunks],
        )
        insert_times_all.append(time.perf_counter() - t)

        # Query with question text
        q_text = rec["question"]
        qemb = None
        if use_daemon:
            qemb = _daemon_embed(q_text)
        if qemb is None:
            qemb = embedder.encode([q_text], show_progress_bar=False).tolist()[0]

        fetch_n = 50 if use_rerank else max(K_VALUES)
        qt = time.perf_counter()
        qr = _query(qemb, fetch_n)
        query_latencies.append((time.perf_counter() - qt) * 1000)

        top_texts_raw = _top_texts(qr, fetch_n)
        if use_rerank and top_texts_raw:
            candidates = [
                {"id": i, "text": txt, "score": 0.5, "uri": None, "title": None}
                for i, txt in enumerate(top_texts_raw)
            ]
            reranked = _daemon_rerank(rec["question"], candidates, max(K_VALUES))
            if reranked:
                top_texts_raw = [str(h.get("text", "")) for h in reranked]

        # Check if top-K texts contain the answer
        answer_lc = rec["answer"].lower().strip()
        for k in K_VALUES:
            top = top_texts_raw[:k]
            if any(answer_lc[:30] in t.lower() for t in top):
                hits_at[k] += 1

    n = len(records)
    result["total_chunks"] = total_chunks_all
    result["avg_chunks_per_rec"] = round(total_chunks_all / max(n, 1))
    result["insert_ops_per_s"] = round(total_chunks_all / max(sum(insert_times_all), 1e-6))
    for k in K_VALUES:
        result[f"recall_at_{k}"] = round(hits_at[k] / n, 4) if n else 0.0
    result["query_p50_ms"] = round(pct(query_latencies, 50), 3)
    result["query_p99_ms"] = round(pct(query_latencies, 99), 3)
    result["rss_mb"] = round(rss_mb(), 1)
    result["wall_s"] = round(time.perf_counter() - t0_wall, 2)
    return result


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--data", default=None)
    parser.add_argument("--full", action="store_true",
                        help="Use lme_s_500.json (must be present)")
    parser.add_argument("--backend", default="both",
                        choices=["chroma", "synapse", "both"])
    parser.add_argument("--heldout", action="store_true",
                        help="Held-out eval: 40 train / 10 test (honest R@K)")
    parser.add_argument("--n-train", type=int, default=40)
    parser.add_argument("--n-test", type=int, default=10)
    parser.add_argument("--sweep", action="store_true",
                        help="Session-level chunking + hybrid query (Track C)")
    parser.add_argument("--rerank", action="store_true",
                        help="After FTS+vec top-50, rerank via synapsed Request::Rerank (daemon must be running)")
    args = parser.parse_args()

    if args.data:
        data_path = pathlib.Path(args.data)
    elif args.full:
        data_path = DATA_500
        if not data_path.exists():
            print(f"BLOCKER: {data_path} not found.")
            print("LongMemEval-500 requires HF login (gated dataset).")
            print("Falling back to lme_s_50.json with held-out split.")
            data_path = DATA_50
            args.heldout = True
    else:
        data_path = DATA_50

    records = load_records(data_path)
    print(f"Loaded {len(records)} records from {data_path}")

    embedder = _get_embedder()
    print("Embedder: all-MiniLM-L6-v2 (384-dim)")

    backends = ["chroma", "synapse"] if args.backend == "both" else [args.backend]
    results = []

    if args.sweep:
        # Per-record eval: use first n_train+n_test records (or all 50)
        eval_recs = records[:args.n_train + args.n_test]
        daemon_up = _daemon_alive()
        try:
            import msgpack
            _has_msgpack = True
        except ImportError:
            _has_msgpack = False
        use_daemon = daemon_up and _has_msgpack
        print(f"Sweep eval: {len(eval_recs)} records (per-record index+query)")
        print(f"Daemon embed for queries: {'YES (/tmp/synapse.sock)' if use_daemon else 'NO (SBERT fallback)'}")
        for bname in backends:
            store_dir = tempfile.mkdtemp(prefix=f"mp-sweep-{bname}-")
            print(f"\nRunning {bname} (sweep) ...")
            r = run_backend_sweep(bname, eval_recs, embedder, store_dir, use_daemon=use_daemon, use_rerank=args.rerank)
            results.append(r)
            if r.get("error"):
                print(f"  ERROR: {r['error']}")
            else:
                print(
                    f"  chunks={r.get('total_chunks','?')} avg={r.get('avg_chunks_per_rec','?')}/rec "
                    f"insert={r['insert_ops_per_s']} ops/s "
                    f"p50={r['query_p50_ms']}ms "
                    f"R@5={r.get('recall_at_5','?')} R@10={r.get('recall_at_10','?')} "
                    f"rss={r['rss_mb']}MB wall={r['wall_s']}s"
                )
            shutil.rmtree(store_dir, ignore_errors=True)
    elif args.heldout:
        train, test = make_splits(records, args.n_train, args.n_test)
        print(f"Held-out split: {len(train)} train / {len(test)} test")
        for bname in backends:
            store_dir = tempfile.mkdtemp(prefix=f"mp-heldout-{bname}-")
            print(f"\nRunning {bname} (held-out) ...")
            r = run_backend_heldout(bname, train, test, embedder, store_dir, use_rerank=args.rerank)
            results.append(r)
            if r.get("error"):
                print(f"  ERROR: {r['error']}")
            else:
                print(
                    f"  insert={r['insert_ops_per_s']} ops/s "
                    f"p50={r['query_p50_ms']}ms p99={r['query_p99_ms']}ms "
                    f"R@5={r.get('recall_at_5','?')} R@10={r.get('recall_at_10','?')} "
                    f"rss={r['rss_mb']}MB disk={r['disk_mb']}MB wall={r['wall_s']}s"
                )
            shutil.rmtree(store_dir, ignore_errors=True)
    else:
        for bname in backends:
            store_dir = tempfile.mkdtemp(prefix=f"mp-bench-{bname}-")
            print(f"\nRunning {bname} (self-match) ...")
            r = run_backend_selfmatch(bname, records, embedder, store_dir)
            results.append(r)
            if r.get("error"):
                print(f"  ERROR: {r['error']}")
            else:
                print(
                    f"  insert={r['insert_ops_per_s']} ops/s "
                    f"p50={r['query_p50_ms']}ms p99={r['query_p99_ms']}ms "
                    f"R@5={r.get('recall_at_5','?')} "
                    f"rss={r['rss_mb']}MB disk={r['disk_mb']}MB wall={r['wall_s']}s"
                )
            shutil.rmtree(store_dir, ignore_errors=True)

    out_path = HERE / "results.json"
    with open(out_path, "w") as f:
        json.dump(results, f, indent=2)
    print(f"\nResults written to {out_path}")


if __name__ == "__main__":
    main()
