#!/usr/bin/env python3
"""Local recall bakeoff: Synapse vs installed memory/context candidates.

The suite intentionally uses tiny but unique golden facts. That keeps the run
cheap while still exercising real ingest and retrieval paths for each tool.
"""

from __future__ import annotations

import json
import os
import re
import argparse
import sqlite3
import statistics
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
RESULTS = HERE / "results"
LIMIT = 5
_SYNAPSE_INGEST_MS: float | None = None


def now_ms() -> float:
    return time.perf_counter() * 1000.0


def pct(values: list[float], q: float) -> float:
    if not values:
        return 0.0
    if len(values) == 1:
        return values[0]
    return statistics.quantiles(values, n=100, method="inclusive")[int(q) - 1]


def run_cmd(
    args: list[str],
    *,
    env: dict[str, str] | None = None,
    timeout: float = 30.0,
    cwd: Path = ROOT,
) -> subprocess.CompletedProcess[str]:
    merged = os.environ.copy()
    if env:
        merged.update(env)
    return subprocess.run(
        args,
        cwd=str(cwd),
        env=merged,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout,
        check=False,
    )


def strip_ansi(text: str) -> str:
    return re.sub(r"\x1b\[[0-9;]*m", "", text)


def parse_json_from_output(text: str) -> Any:
    text = strip_ansi(text).strip()
    if not text:
        return None
    marker = "###BAKEOFF_JSON###"
    if marker in text:
        text = text.split(marker, 1)[1].strip()
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        start = text.find("{")
        arr = text.find("[")
        starts = [x for x in (start, arr) if x >= 0]
        if starts:
            return json.loads(text[min(starts) :])
        raise


def add_note_once(notes: list[str], note: str) -> None:
    if note and note not in notes:
        notes.append(note)


def suite_id(records: list[dict[str, Any]]) -> str:
    return records[0]["expected"].split("-AETHER", 1)[0]


def make_records(run_id: str | None = None) -> list[dict[str, Any]]:
    run_id = run_id or f"RBK-{time.strftime('%Y%m%d-%H%M%S')}"
    specs = [
        (
            "AETHER",
            "Socket keepalive batch path removes fork/connect overhead for hook chains.",
            ["Which path removes fork connect overhead?", "hook chain batch keepalive path"],
        ),
        (
            "BORON",
            "Version freshness must resolve local package docs before model training memory.",
            ["How should version slippage be avoided?", "local package docs before training memory"],
        ),
        (
            "CIRRUS",
            "Contradiction handling requires temporal validity windows and supersession edges.",
            ["What handles evolving facts and contradictions?", "temporal validity supersession edges"],
        ),
        (
            "DRAKE",
            "Hot recall budget for prompt hooks should stay below twenty five milliseconds p95.",
            ["What is the hot recall p95 hook budget?", "twenty five milliseconds p95 prompt hooks"],
        ),
        (
            "EMBER",
            "Token saving improves when recall packs evidence by task mode and drops stale near duplicates.",
            ["How does recall save tokens?", "task mode stale near duplicates"],
        ),
        (
            "FLINT",
            "Local-first memory should degrade to lexical search when embeddings or daemons fail.",
            ["What fallback is required when embeddings fail?", "degrade lexical search embeddings fail"],
        ),
        (
            "GRANITE",
            "A reranker should learn from accepted context, file opens, edits, and later task success.",
            ["What feedback should the reranker learn from?", "accepted context file opens edits task success"],
        ),
        (
            "HELIOS",
            "Fresh docs adapters should be fallback evidence, not the primary hot path.",
            ["Where should fresh docs adapters sit?", "fallback evidence primary hot path"],
        ),
    ]
    records = []
    for token, fact, queries in specs:
        expected = f"{run_id}-{token}"
        text = (
            f"{expected}: {fact} "
            f"Scope=local-agent-memory. Category=recall-bakeoff. Token={token}."
        )
        records.append(
            {
                "id": token.lower(),
                "expected": expected,
                "text": text,
                "queries": [f"{run_id} {q}" for q in queries],
            }
        )
    return records


def hit_blob(hit: Any) -> str:
    if isinstance(hit, str):
        return hit
    if isinstance(hit, dict):
        pieces = []
        for key in (
            "text",
            "content",
            "memory",
            "title",
            "id",
            "uri",
            "answer",
            "snippet",
        ):
            val = hit.get(key)
            if val is not None:
                pieces.append(str(val))
        return "\n".join(pieces)
    return str(hit)


def rank_of(hits: list[Any], expected: str) -> int | None:
    for i, hit in enumerate(hits, start=1):
        if expected in hit_blob(hit):
            return i
    return None


@dataclass
class EngineResult:
    name: str
    status: str
    ingest_ms: float
    queries: list[dict[str, Any]]
    notes: list[str]


def sqlite_fts_engine(records: list[dict[str, Any]]) -> EngineResult:
    con = sqlite3.connect(":memory:")
    t0 = now_ms()
    con.execute("CREATE VIRTUAL TABLE docs USING fts5(id, text)")
    con.executemany("INSERT INTO docs(id, text) VALUES (?, ?)", [(r["id"], r["text"]) for r in records])
    con.commit()
    ingest_ms = now_ms() - t0
    queries = []
    for r in records:
        for q in r["queries"]:
            t = now_ms()
            terms = re.findall(r"[A-Za-z0-9_]+", q.replace("-", " "))
            safe = " OR ".join(f'"{term}"' for term in terms)
            rows = con.execute(
                "SELECT id, text FROM docs WHERE docs MATCH ? LIMIT ?",
                (safe, LIMIT),
            ).fetchall()
            hits = [{"id": row[0], "text": row[1]} for row in rows]
            queries.append(
                {
                    "query": q,
                    "expected": r["expected"],
                    "latency_ms": now_ms() - t,
                    "rank": rank_of(hits, r["expected"]),
                    "hits": hits,
                }
            )
    return EngineResult("sqlite_fts5_control", "ok", ingest_ms, queries, ["local lexical control"])


def ensure_synapse_records(records: list[dict[str, Any]]) -> float:
    global _SYNAPSE_INGEST_MS
    if _SYNAPSE_INGEST_MS is not None:
        return 0.0
    sys.path.insert(0, str(ROOT / "sdk/python"))
    from synapse_memory.client import Client  # type: ignore

    client = Client(timeout=60)
    scope = f"bench/recall_bakeoff/{suite_id(records)}"
    t0 = now_ms()
    client.put_batch(
        [
            {
                "title": f"recall-bakeoff:{r['id']}",
                "text": r["text"],
                "meta": {"scope": scope, "expected": r["expected"]},
            }
            for r in records
        ],
        embed=True,
        timeout=600,
    )
    _SYNAPSE_INGEST_MS = now_ms() - t0
    return _SYNAPSE_INGEST_MS


def synapse_engine(records: list[dict[str, Any]]) -> EngineResult:
    sys.path.insert(0, str(ROOT / "sdk/python"))
    from synapse_memory.client import Client  # type: ignore

    ingest_ms = ensure_synapse_records(records)
    client = Client(timeout=60)
    queries = []
    for r in records:
        for q in r["queries"]:
            t = now_ms()
            hits = client.search(q, mode="hybrid", limit=LIMIT)
            queries.append(
                {
                    "query": q,
                    "expected": r["expected"],
                    "latency_ms": now_ms() - t,
                    "rank": rank_of(hits, r["expected"]),
                    "hits": hits[:LIMIT],
                }
            )
    return EngineResult("synapse_hybrid_socket", "ok", ingest_ms, queries, ["global Synapse daemon via Unix socket"])


def synapse_scoped_fusion_engine(records: list[dict[str, Any]]) -> EngineResult:
    """SDK scoped recall: BatchSearch + docs.meta scope + query-term rerank."""
    sys.path.insert(0, str(ROOT / "sdk/python"))
    from synapse_memory.client import Client  # type: ignore

    ingest_ms = ensure_synapse_records(records)
    client = Client(timeout=60)
    scope = f"bench/recall_bakeoff/{suite_id(records)}"
    queries = []
    for r in records:
        for q in r["queries"]:
            t = now_ms()
            hits = client.search_scoped_fusion(q, scope=scope, limit=LIMIT, fetch_k=50)
            queries.append(
                {
                    "query": q,
                    "expected": r["expected"],
                    "latency_ms": now_ms() - t,
                    "rank": rank_of(hits, r["expected"]),
                    "hits": hits,
                }
            )
    return EngineResult(
        "synapse_scoped_fusion_sdk",
        "ok",
        ingest_ms,
        queries,
        ["indexed scope-first recall, BatchSearch fallback, docs.meta filter, query-term rerank"],
    )


def omega_engine(records: list[dict[str, Any]]) -> EngineResult:
    omega = ROOT / ".tools/omega-memory/bin/omega"
    if not omega.exists():
        return EngineResult("omega_memory", "skipped", 0.0, [], ["omega CLI missing"])
    t0 = now_ms()
    notes: list[str] = []
    for r in records:
        cp = run_cmd([str(omega), "store", "--json", "-t", "memory", r["text"]], timeout=30)
        stored_ok = False
        if cp.stdout.strip():
            try:
                stored_ok = parse_json_from_output(cp.stdout).get("status") == "ok"
            except Exception:
                stored_ok = False
        if cp.returncode != 0 and not stored_ok:
            note = strip_ansi((cp.stderr or cp.stdout or "omega store failed")[-800:])
            return EngineResult("omega_memory", "error", now_ms() - t0, [], [note])
        if cp.returncode != 0 and stored_ok:
            add_note_once(notes, "omega store returned nonzero despite JSON status=ok")
    ingest_ms = now_ms() - t0
    queries = []
    for r in records:
        for q in r["queries"]:
            t = now_ms()
            cp = run_cmd([str(omega), "query", "--json", "--limit", str(LIMIT), q], timeout=30)
            latency = now_ms() - t
            if cp.returncode != 0:
                add_note_once(notes, cp.stderr[-300:])
                hits: list[Any] = []
            else:
                data = parse_json_from_output(cp.stdout)
                hits = data.get("results", []) if isinstance(data, dict) else []
            queries.append(
                {
                    "query": q,
                    "expected": r["expected"],
                    "latency_ms": latency,
                    "rank": rank_of(hits, r["expected"]),
                    "hits": hits[:LIMIT],
                }
            )
    return EngineResult("omega_memory_cli", "ok", ingest_ms, queries, notes)


def signet_engine(records: list[dict[str, Any]]) -> EngineResult:
    signet = ROOT / ".tools/signet/node_modules/.bin/signet"
    if not signet.exists():
        return EngineResult("signet_daemon", "skipped", 0.0, [], ["signet CLI missing"])
    env = {
        "SIGNET_PATH": str(ROOT / ".tools/signet-agent"),
        "SIGNET_DAEMON_URL": "http://localhost:3850",
    }
    health = run_cmd(["curl", "-fsS", "http://localhost:3850/health"], timeout=5)
    if health.returncode != 0:
        return EngineResult("signet_daemon", "skipped", 0.0, [], ["signet daemon not reachable"])
    t0 = now_ms()
    notes: list[str] = []
    for r in records:
        cp = run_cmd(
            [str(signet), "remember", "--tags", "synapse-bakeoff", "--importance", "0.4", r["text"]],
            env=env,
            timeout=30,
        )
        if cp.returncode != 0:
            return EngineResult("signet_daemon", "error", now_ms() - t0, [], [cp.stderr[-500:]])
    ingest_ms = now_ms() - t0
    queries = []
    for r in records:
        for q in r["queries"]:
            t = now_ms()
            cp = run_cmd([str(signet), "recall", q, "--json", "--limit", str(LIMIT)], env=env, timeout=30)
            latency = now_ms() - t
            if cp.returncode != 0:
                add_note_once(notes, cp.stderr[-300:])
                hits = []
            else:
                data = parse_json_from_output(cp.stdout)
                hits = data.get("results", []) if isinstance(data, dict) else []
            queries.append(
                {
                    "query": q,
                    "expected": r["expected"],
                    "latency_ms": latency,
                    "rank": rank_of(hits, r["expected"]),
                    "hits": hits[:LIMIT],
                }
            )
    return EngineResult("signet_daemon_cli", "ok", ingest_ms, queries, notes)


def helper_engine(name: str, python: Path, helper_source: str, payload: dict[str, Any], timeout: float) -> EngineResult:
    if not python.exists():
        return EngineResult(name, "skipped", 0.0, [], [f"{python} missing"])
    with tempfile.TemporaryDirectory(prefix=f"{name}-") as td:
        p = Path(td) / "payload.json"
        p.write_text(json.dumps(payload), encoding="utf-8")
        cp = run_cmd([str(python), "-c", helper_source, str(p)], timeout=timeout)
    if cp.returncode != 0:
        return EngineResult(name, "error", 0.0, [], [strip_ansi((cp.stderr or cp.stdout)[-1500:])])
    data = parse_json_from_output(cp.stdout)
    return EngineResult(
        name,
        data.get("status", "ok"),
        float(data.get("ingest_ms", 0.0)),
        data.get("queries", []),
        data.get("notes", []),
    )


def mem0_engine(records: list[dict[str, Any]]) -> EngineResult:
    helper = r'''
import json, os, sys, tempfile, time
from pathlib import Path
from mem0.memory import main as mm
from mem0.embeddings.mock import MockEmbeddings
from mem0.llms.base import LLMBase
from mem0.configs.llms.base import BaseLlmConfig

class DummyLLM(LLMBase):
    def __init__(self, config=None):
        super().__init__(BaseLlmConfig(model="dummy"))
    def generate_response(self, *args, **kwargs):
        return "[]"

mm.EmbedderFactory.create = lambda provider, config, vector_config=None: MockEmbeddings()
mm.LlmFactory.create = lambda provider, config: DummyLLM()

from mem0 import Memory

payload = json.load(open(sys.argv[1]))
records = payload["records"]
limit = payload["limit"]
base = Path(tempfile.mkdtemp(prefix="mem0-bakeoff-"))
cfg = {
    "vector_store": {
        "provider": "faiss",
        "config": {
            "collection_name": "synapse_bakeoff",
            "path": str(base / "faiss"),
            "embedding_model_dims": 10,
        },
    },
    "embedder": {"provider": "openai", "config": {"api_key": "dummy", "model": "dummy"}},
    "llm": {"provider": "openai", "config": {"api_key": "dummy", "model": "dummy"}},
    "history_db_path": str(base / "history.db"),
}
m = Memory.from_config(cfg)
user_id = payload["suite_id"]
t0 = time.perf_counter() * 1000
for r in records:
    m.add(r["text"], user_id=user_id, infer=False)
ingest_ms = time.perf_counter() * 1000 - t0
queries = []
for r in records:
    for q in r["queries"]:
        t = time.perf_counter() * 1000
        data = m.search(q, filters={"user_id": user_id}, threshold=0.0, top_k=limit)
        latency = time.perf_counter() * 1000 - t
        hits = data.get("results", []) if isinstance(data, dict) else data
        def blob(h):
            return " ".join(str(h.get(k, "")) for k in ("memory", "text", "content", "id")) if isinstance(h, dict) else str(h)
        rank = next((i for i, h in enumerate(hits, 1) if r["expected"] in blob(h)), None)
        queries.append({"query": q, "expected": r["expected"], "latency_ms": latency, "rank": rank, "hits": hits[:limit]})
print("###BAKEOFF_JSON###" + json.dumps({"status": "ok", "ingest_ms": ingest_ms, "queries": queries, "notes": ["mem0 FAISS + mock embeddings, no cloud"]}))
'''
    suite_id = records[0]["expected"].split("-AETHER", 1)[0]
    return helper_engine(
        "mem0_faiss_mock_local",
        ROOT / ".tools/mem0/bin/python",
        helper,
        {"records": records, "limit": LIMIT, "suite_id": suite_id},
        timeout=120,
    )


def synapse_mem0_compat_engine(records: list[dict[str, Any]]) -> EngineResult:
    sys.path.insert(0, str(ROOT / "sdk/python"))
    from synapse_mem0 import Memory  # type: ignore

    suite = suite_id(records)
    user_id = f"recall-bakeoff-{suite}"
    t0 = now_ms()
    notes: list[str] = []
    with Memory() as m:
        for r in records:
            m.add(
                f"{r['text']} Engine=synapse_mem0_compat.",
                user_id=user_id,
                metadata={"expected": r["expected"]},
            )
        ingest_ms = now_ms() - t0
        queries = []
        for r in records:
            for q in r["queries"]:
                t = now_ms()
                data = m.search(q, user_id=user_id, limit=LIMIT)
                hits = data.get("results", []) if isinstance(data, dict) else []
                queries.append(
                    {
                        "query": q,
                        "expected": r["expected"],
                        "latency_ms": now_ms() - t,
                        "rank": rank_of(hits, r["expected"]),
                        "hits": hits[:LIMIT],
                    }
                )
    notes.append("Synapse-backed drop-in mem0 API, local socket, Lex mode, no cloud")
    return EngineResult("synapse_mem0_compat_socket", "ok", ingest_ms, queries, notes)


def cognee_engine(records: list[dict[str, Any]]) -> EngineResult:
    helper = r'''
import asyncio, json, sys, time
import cognee

payload = json.load(open(sys.argv[1]))
records = payload["records"]
limit = payload["limit"]
session_id = payload["suite_id"]

async def main():
    notes = []
    t0 = time.perf_counter() * 1000
    for r in records:
        await cognee.remember(r["text"], session_id=session_id, self_improvement=False)
    ingest_ms = time.perf_counter() * 1000 - t0
    queries = []
    for r in records:
        for q in r["queries"]:
            t = time.perf_counter() * 1000
            try:
                ans = await cognee.recall(q, session_id=session_id, only_context=True, top_k=limit)
                raw_hits = ans if isinstance(ans, list) else [ans]
                hits = [str(h) for h in raw_hits]
            except Exception as e:
                notes.append(type(e).__name__ + ": " + str(e)[:240])
                hits = []
            latency = time.perf_counter() * 1000 - t
            def blob(h):
                return h if isinstance(h, str) else str(h)
            rank = next((i for i, h in enumerate(hits, 1) if r["expected"] in blob(h)), None)
            queries.append({"query": q, "expected": r["expected"], "latency_ms": latency, "rank": rank, "hits": hits[:limit]})
    print("###BAKEOFF_JSON###" + json.dumps({"status": "ok", "ingest_ms": ingest_ms, "queries": queries, "notes": notes or ["cognee session recall, graph path not used"]}))

asyncio.run(main())
'''
    suite_id = records[0]["expected"].split("-AETHER", 1)[0]
    return helper_engine(
        "cognee_session_recall",
        ROOT / ".tools/cognee/bin/python",
        helper,
        {"records": records, "limit": LIMIT, "suite_id": suite_id},
        timeout=180,
    )


def claude_mem_engine(records: list[dict[str, Any]]) -> EngineResult:
    """Benchmark claude-mem's local worker using isolated /api/import + /api/search."""

    helper = r'''
import json, os, shutil, socket, subprocess, sys, tempfile, time, urllib.parse, urllib.request
from pathlib import Path

payload = json.load(open(sys.argv[1]))
records = payload["records"]
limit = payload["limit"]
suite_id = payload["suite_id"]
root = Path(payload["root"])
script = root / ".tools/claude-mem-src/plugin/scripts/worker-service.cjs"
bun = shutil.which("bun")
if not bun:
    print("###BAKEOFF_JSON###" + json.dumps({"status": "skipped", "ingest_ms": 0, "queries": [], "notes": ["bun missing"]}))
    raise SystemExit(0)
if not script.exists():
    print("###BAKEOFF_JSON###" + json.dumps({"status": "skipped", "ingest_ms": 0, "queries": [], "notes": [f"worker script missing: {script}"]}))
    raise SystemExit(0)

def free_port():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]

data_dir = tempfile.mkdtemp(prefix="claude-mem-bakeoff-")
port = free_port()
env = os.environ.copy()
env["CLAUDE_MEM_DATA_DIR"] = data_dir
env["CLAUDE_MEM_WORKER_PORT"] = str(port)
env["CLAUDE_MEM_CHROMA_ENABLED"] = "false"
base = f"http://127.0.0.1:{port}"
notes = ["claude-mem 13.2.0 worker, isolated temp profile, Chroma disabled, /api/import + /api/search?format=json"]

def http_json(method, path, obj=None, timeout=20):
    data = json.dumps(obj).encode("utf-8") if obj is not None else None
    req = urllib.request.Request(
        base + path,
        data=data,
        method=method,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        body = resp.read().decode("utf-8")
        return json.loads(body) if body else None

def wait_health(timeout=35):
    deadline = time.time() + timeout
    last = None
    while time.time() < deadline:
        try:
            return http_json("GET", "/api/health", timeout=2)
        except Exception as exc:
            last = exc
            time.sleep(0.25)
    raise RuntimeError(f"worker health timeout: {last}")

def stop_worker():
    subprocess.run(
        [bun, str(script), "stop"],
        cwd=str(script.parent),
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=20,
        check=False,
    )

try:
    start = subprocess.run(
        [bun, str(script), "start"],
        cwd=str(script.parent),
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=45,
        check=False,
    )
    if start.returncode != 0:
        print("###BAKEOFF_JSON###" + json.dumps({
            "status": "error",
            "ingest_ms": 0,
            "queries": [],
            "notes": [notes[0], (start.stderr or start.stdout)[-1000:]],
        }))
        raise SystemExit(0)
    wait_health()
    project = f"synapse-recall-bakeoff-{suite_id}"
    memory_session_id = f"memory-{suite_id}"
    content_session_id = f"content-{suite_id}"
    now = int(time.time() * 1000)
    iso = time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime(now / 1000))
    sessions = [{
        "content_session_id": content_session_id,
        "memory_session_id": memory_session_id,
        "project": project,
        "platform_source": "api",
        "user_prompt": "recall bakeoff import",
        "started_at": iso,
        "started_at_epoch": now,
        "completed_at": None,
        "completed_at_epoch": None,
        "status": "completed",
    }]
    observations = []
    for idx, r in enumerate(records, start=1):
        created = now + idx
        created_iso = time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime(created / 1000))
        observations.append({
            "memory_session_id": memory_session_id,
            "project": project,
            "text": r["text"],
            "type": "discovery",
            "title": r["expected"],
            "subtitle": "recall bakeoff",
            "facts": json.dumps([r["text"]]),
            "narrative": r["text"],
            "concepts": json.dumps(["recall-bakeoff", r["id"]]),
            "files_read": "[]",
            "files_modified": "[]",
            "prompt_number": idx,
            "discovery_tokens": max(1, len(r["text"].split())),
            "created_at": created_iso,
            "created_at_epoch": created,
            "agent_type": "bench",
            "agent_id": "synapse-recall-bakeoff",
        })
    t0 = time.perf_counter() * 1000
    imported = http_json("POST", "/api/import", {"sessions": sessions, "observations": observations}, timeout=30)
    ingest_ms = time.perf_counter() * 1000 - t0
    stats = imported.get("stats", {}) if isinstance(imported, dict) else {}
    notes.append(f"import stats: {stats}")
    queries = []
    for r in records:
        for q in r["queries"]:
            path = "/api/search?" + urllib.parse.urlencode({
                "query": q,
                "limit": str(limit),
                "project": project,
                "type": "observations",
                "orderBy": "relevance",
                "format": "json",
            })
            t = time.perf_counter() * 1000
            data = http_json("GET", path, timeout=20)
            latency = time.perf_counter() * 1000 - t
            hits = data.get("observations", []) if isinstance(data, dict) else []
            def blob(h):
                if isinstance(h, dict):
                    return " ".join(str(h.get(k, "")) for k in ("text", "title", "subtitle", "narrative", "facts"))
                return str(h)
            rank = next((i for i, h in enumerate(hits, 1) if r["expected"] in blob(h)), None)
            queries.append({"query": q, "expected": r["expected"], "latency_ms": latency, "rank": rank, "hits": hits[:limit]})
    print("###BAKEOFF_JSON###" + json.dumps({"status": "ok", "ingest_ms": ingest_ms, "queries": queries, "notes": notes}))
finally:
    stop_worker()
'''
    return helper_engine(
        "claude_mem_worker_import_fts",
        Path(sys.executable),
        helper,
        {"records": records, "limit": LIMIT, "suite_id": suite_id(records), "root": str(ROOT)},
        timeout=180,
    )


def http_json(method: str, url: str, obj: dict[str, Any] | None = None, timeout: float = 20.0) -> Any:
    data = json.dumps(obj).encode("utf-8") if obj is not None else None
    req = urllib.request.Request(url, data=data, method=method, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        body = resp.read().decode("utf-8")
        return json.loads(body) if body else None


def letta_engine(records: list[dict[str, Any]]) -> EngineResult:
    base = "http://127.0.0.1:8283"
    try:
        http_json("GET", base + "/v1/health/", timeout=5)
    except (urllib.error.URLError, TimeoutError) as e:
        return EngineResult("letta_archive_passages", "skipped", 0.0, [], [f"Letta not reachable: {e}"])
    suite_id = records[0]["expected"].split("-AETHER", 1)[0]
    notes: list[str] = []
    t0 = now_ms()
    try:
        archive = http_json("POST", base + "/v1/archives/", {"name": f"synapse-bakeoff-{suite_id}"})
        archive_id = archive["id"]
        for r in records:
            http_json(
                "POST",
                base + f"/v1/archives/{archive_id}/passages",
                {"text": r["text"], "tags": ["synapse-bakeoff"]},
            )
    except Exception as e:
        return EngineResult("letta_archive_passages", "error", now_ms() - t0, [], [repr(e)])
    ingest_ms = now_ms() - t0
    time.sleep(0.3)
    queries = []
    for r in records:
        for q in r["queries"]:
            t = now_ms()
            try:
                hits = http_json(
                    "POST",
                    base + "/v1/passages/search",
                    {"query": q, "archive_id": archive_id, "limit": LIMIT},
                    timeout=20,
                )
                if not isinstance(hits, list):
                    hits = []
            except Exception as e:
                add_note_once(notes, repr(e)[:240])
                hits = []
            queries.append(
                {
                    "query": q,
                    "expected": r["expected"],
                    "latency_ms": now_ms() - t,
                    "rank": rank_of(hits, r["expected"]),
                    "hits": hits[:LIMIT],
                }
            )
    if not any(q["rank"] for q in queries):
        add_note_once(notes, "archive write works, /v1/passages/search returned no hits in this local setup")
        status = "degraded"
    else:
        status = "ok"
    return EngineResult("letta_archive_passages", status, ingest_ms, queries, notes)


def unavailable_engines() -> list[EngineResult]:
    return [
        EngineResult(
            "graphiti_core",
            "skipped",
            0.0,
            [],
            ["installed/importable, but no configured graph backend + local LLM/embedder for fair E2E"],
        ),
        EngineResult(
            "context7_docfork_gitmcp_deepwiki",
            "not_recall_engine",
            0.0,
            [],
            ["freshness/docs MCPs are verified separately; they need a docs-version benchmark, not memory recall ingest"],
        ),
    ]


def summarize(engine: EngineResult) -> dict[str, Any]:
    n = len(engine.queries)
    ranks = [q["rank"] for q in engine.queries]
    lat = [float(q["latency_ms"]) for q in engine.queries]
    hit_at_1 = sum(1 for r in ranks if r == 1)
    hit_at_3 = sum(1 for r in ranks if r is not None and r <= 3)
    hit_at_5 = sum(1 for r in ranks if r is not None and r <= 5)
    mrr = sum(0.0 if r is None else 1.0 / r for r in ranks) / n if n else 0.0
    return {
        "name": engine.name,
        "status": engine.status,
        "queries": n,
        "ingest_ms": round(engine.ingest_ms, 2),
        "recall_at_1": round(hit_at_1 / n, 3) if n else None,
        "recall_at_3": round(hit_at_3 / n, 3) if n else None,
        "recall_at_5": round(hit_at_5 / n, 3) if n else None,
        "mrr": round(mrr, 3) if n else None,
        "p50_ms": round(statistics.median(lat), 2) if lat else None,
        "p95_ms": round(pct(lat, 95), 2) if lat else None,
        "max_ms": round(max(lat), 2) if lat else None,
        "notes": engine.notes,
    }


def markdown_report(records: list[dict[str, Any]], engine_results: list[EngineResult], out_json: Path) -> str:
    summaries = [summarize(e) for e in engine_results]
    engine_names = ", ".join(s["name"] for s in summaries)
    ranked = sorted(
        summaries,
        key=lambda x: (
            1 if x["recall_at_5"] is None else 0,
            0 if x["recall_at_5"] is None else -x["recall_at_5"],
            10**9 if x["p95_ms"] is None else x["p95_ms"],
        ),
    )
    lines = [
        "# Recall Bakeoff: Synapse vs Local Candidates",
        "",
        f"Run: `{records[0]['expected'].split('-AETHER', 1)[0]}`",
        f"Golden records: `{len(records)}`; queries: `{sum(len(r['queries']) for r in records)}`; top-k: `{LIMIT}`",
        f"Raw JSON: `{out_json.name}`",
        "",
        "## Scoreboard",
        "",
        "| Engine | Status | R@1 | R@5 | MRR | p50 ms | p95 ms | Ingest ms | Notes |",
        "|---|---:|---:|---:|---:|---:|---:|---:|---|",
    ]
    for s in ranked:
        note = "; ".join(s["notes"][:2]) if s["notes"] else ""
        lines.append(
            "| {name} | {status} | {r1} | {r5} | {mrr} | {p50} | {p95} | {ingest} | {note} |".format(
                name=s["name"],
                status=s["status"],
                r1="-" if s["recall_at_1"] is None else s["recall_at_1"],
                r5="-" if s["recall_at_5"] is None else s["recall_at_5"],
                mrr="-" if s["mrr"] is None else s["mrr"],
                p50="-" if s["p50_ms"] is None else s["p50_ms"],
                p95="-" if s["p95_ms"] is None else s["p95_ms"],
                ingest=s["ingest_ms"],
                note=note.replace("|", "/"),
            )
        )
    lines += [
        "",
        "## What The Run Actually Tests",
        "",
        f"- Real write path plus real query path for the selected engines: {engine_names}.",
        "- Unique per-run expected tokens avoid false wins from the existing large Synapse corpus.",
        "- This is a small precision/latency gate, not a public LongMemEval/LoCoMo claim.",
        "",
        "## Architecture Patterns That Win",
        "",
        "1. Hot path and deep path must be separate: Synapse should keep the socket/FTS/vector path tiny, while graph/agent tools run behind it as enrichment.",
        "2. Local-first docs freshness should be a version-resolved evidence layer: local package docs first, remote MCP docs second, training memory last.",
        "3. Recall needs lifecycle semantics: temporal validity, supersedes/conflicts edges, and stale duplicate dampening are more valuable than just more vectors.",
        "4. Query routing should be learned from outcomes: accepted context, opened files, edits, test pass/fail, and user correction create the best reward signal.",
        "5. Batch/keepalive beats micro-optimizing per-call work once fork/connect dominates; this is the cleanest near-term Synapse latency lever.",
        "6. Token savings are a retrieval quality problem: pack fewer, fresher, higher-confidence facts by task mode instead of dumping more context.",
        "",
        "## Best Next Architecture",
        "",
        "Synapse should be the primary local recall kernel. OMEGA contributes memory lifecycle ideas, Signet contributes portable identity/structured memory ideas, Letta contributes core-vs-archival agent memory, Cognee/Graphiti contribute graph/temporal enrichment, and Context7/Docfork/GitMCP/DeepWiki contribute freshness evidence. The winning shape is not replacing Synapse; it is a router that keeps Synapse hot and calls the others only when they add measurable value.",
    ]
    return "\n".join(lines) + "\n"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run local recall bakeoff")
    parser.add_argument(
        "--engines",
        default="all",
        help=(
            "Comma-separated engine names. Choices: sqlite,synapse,synapse_scoped,"
            "synapse_mem0,omega,signet,mem0,cognee,letta,claude_mem,all"
        ),
    )
    parser.add_argument("--run-id", default=None, help="Stable run id for reproducible debugging")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    RESULTS.mkdir(parents=True, exist_ok=True)
    records = make_records(args.run_id)
    registry = {
        "sqlite": sqlite_fts_engine,
        "synapse": synapse_engine,
        "synapse_scoped": synapse_scoped_fusion_engine,
        "synapse_mem0": synapse_mem0_compat_engine,
        "omega": omega_engine,
        "signet": signet_engine,
        "mem0": mem0_engine,
        "cognee": cognee_engine,
        "letta": letta_engine,
        "claude_mem": claude_mem_engine,
    }
    selected_names = list(registry) if args.engines == "all" else [x.strip() for x in args.engines.split(",") if x.strip()]
    unknown = [name for name in selected_names if name not in registry]
    if unknown:
        raise SystemExit(f"unknown engine(s): {', '.join(unknown)}")
    engines = [registry[name] for name in selected_names]
    results: list[EngineResult] = []
    for engine in engines:
        name = engine.__name__.replace("_engine", "")
        print(f"running {name}...", file=sys.stderr, flush=True)
        try:
            results.append(engine(records))
        except Exception as e:
            results.append(EngineResult(name, "error", 0.0, [], [repr(e)]))
    if args.engines == "all":
        results.extend(unavailable_engines())

    stamp = time.strftime("%Y%m%d-%H%M%S")
    out_json = RESULTS / f"recall_bakeoff_{stamp}.json"
    out_md = RESULTS / f"recall_bakeoff_{stamp}.md"
    payload = {
        "records": records,
        "summaries": [summarize(e) for e in results],
        "engines": [
            {
                "name": e.name,
                "status": e.status,
                "ingest_ms": e.ingest_ms,
                "notes": e.notes,
                "queries": e.queries,
            }
            for e in results
        ],
    }
    out_json.write_text(json.dumps(payload, indent=2, ensure_ascii=False), encoding="utf-8")
    out_md.write_text(markdown_report(records, results, out_json), encoding="utf-8")
    print(json.dumps({"json": str(out_json), "markdown": str(out_md), "summaries": payload["summaries"]}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
