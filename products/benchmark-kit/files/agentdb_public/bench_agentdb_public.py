#!/usr/bin/env python3
"""Public AgentDB benchmark and context-pack tuner.

The goal is deliberately narrow: verify Synapse as an agent-facing local
database, not a generic vector benchmark. The harness measures:

1. scoped recall quality on paraphrased queries,
2. AgentDB context-pack latency,
3. token savings from progressive disclosure,
4. a deterministic Autolearn-style race over context-pack configurations,
5. recall of older relevant memories behind a newer scoped noise tail.
"""

from __future__ import annotations

import argparse
import json
import statistics
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
SDK = ROOT / "sdk" / "python"
RESULTS = Path(__file__).resolve().parent / "results"
if str(SDK) not in sys.path:
    sys.path.insert(0, str(SDK))

from synapse_memory import Client  # noqa: E402


@dataclass(frozen=True)
class Record:
    title: str
    text: str
    queries: tuple[str, ...]


@dataclass(frozen=True)
class Arm:
    name: str
    token_budget: int
    index_k: int
    full_k: int
    snippet_chars: int


ARMS = [
    Arm("speed", token_budget=700, index_k=5, full_k=1, snippet_chars=160),
    Arm("balanced", token_budget=1100, index_k=8, full_k=2, snippet_chars=220),
    Arm("recall", token_budget=2200, index_k=12, full_k=4, snippet_chars=280),
    Arm("token_saver", token_budget=900, index_k=10, full_k=1, snippet_chars=180),
]


def pct(values: list[float], q: float) -> float:
    if not values:
        return 0.0
    if len(values) == 1:
        return values[0]
    return statistics.quantiles(values, n=100, method="inclusive")[int(q) - 1]


def now_ms() -> float:
    return time.perf_counter_ns() / 1_000_000.0


def make_records(run_id: str, repeat: int) -> list[Record]:
    filler = (
        "Agent memory should prefer precise scoped recall, compact first-pass "
        "indexes, source-backed facts, feedback learning, and zero external "
        "services on the hot path. "
    )
    specs = [
        (
            "decision/router",
            "Use scoped fusion as the hot recall path for coding agents.",
            (
                "which recall route should coding agents use",
                "hot path for local agent memory routing",
            ),
        ),
        (
            "decision/progressive-disclosure",
            "Return a compact search index first, then hydrate selected observations only.",
            (
                "how should agent memory save context tokens",
                "progressive disclosure index then full observations",
            ),
        ),
        (
            "decision/freshness",
            "Resolve local package versions and source_uri evidence before model memory.",
            (
                "how does the agent avoid version slippage",
                "local package versions before training memory",
            ),
        ),
        (
            "decision/feedback-rerank",
            "Accepted hits, rejected hits, opened files, edits, and green tests train reranking.",
            (
                "what feedback should train memory reranking",
                "accepted rejected hits edits tests reranker",
            ),
        ),
        (
            "decision/lifecycle",
            "Memories need valid_from, valid_until, confidence, source_uri, and supersession metadata.",
            (
                "which lifecycle metadata makes memory reliable",
                "validity confidence source supersession fields",
            ),
        ),
        (
            "decision/degrade",
            "Agent memory must degrade to lexical scoped search when embeddings are unavailable.",
            (
                "what fallback works without embeddings",
                "degrade lexical scoped search unavailable embedding",
            ),
        ),
        (
            "decision/local-first",
            "The public agent database should be a local single-file store with a Unix socket daemon.",
            (
                "what deployment shape is best for local agents",
                "single file unix socket local agent database",
            ),
        ),
        (
            "decision/graph-behind-hotpath",
            "Graph and temporal enrichment should run behind the hot path, not in front of recall.",
            (
                "where should graph enrichment sit",
                "temporal graph behind hot recall path",
            ),
        ),
    ]
    records = []
    for idx, (title, fact, queries) in enumerate(specs):
        marker = f"{run_id}-AGENTDB-{idx:02d}"
        long_text = (
            f"{marker}. {fact} "
            f"Benchmark title={title}. "
            + (filler * repeat)
            + f" Final evidence marker {marker}."
        )
        records.append(
            Record(
                title=title,
                text=long_text,
                queries=tuple(f"{run_id} {query}" for query in queries),
            )
        )
    return records


def make_noise_items(agent: Any, n: int) -> list[dict[str, Any]]:
    """Newer scoped rows that should not outrank the older target records."""
    items = []
    for idx in range(n):
        items.append({
            "title": f"noise/load-row-{idx:03d}",
            "text": (
                "Synthetic benchmark load row about unrelated weather, cooking, "
                "and calendar notes. It exists to make the relevant observations "
                "older than the recent scoped fetch window."
            ),
            "meta": {
                "schema": agent.schema,
                "scope": agent.scope,
                "agent_id": agent.agent_id,
                "project": agent.project,
                "kind": "noise",
                "tags": ["agentdb", "public-bench", "noise"],
            },
        })
    return items


def rank_of(hits: list[dict[str, Any]], expected_id: int) -> int | None:
    for rank, hit in enumerate(hits, 1):
        if int(hit.get("id", -1)) == expected_id:
            return rank
    return None


def metrics_from_ranks(ranks: list[int | None]) -> dict[str, float]:
    n = max(len(ranks), 1)
    return {
        "r_at_1": round(sum(1 for rank in ranks if rank == 1) / n, 4),
        "r_at_5": round(sum(1 for rank in ranks if rank and rank <= 5) / n, 4),
        "mrr": round(sum((1 / rank) for rank in ranks if rank) / n, 4),
    }


def run_benchmark(args: argparse.Namespace) -> dict[str, Any]:
    run_id = args.run_id or f"ADB-{time.strftime('%Y%m%d-%H%M%S')}"
    project = args.project or f"agentdb-public-{run_id}"
    client = Client(sock_path=args.sock)
    if not client.ping():
        raise RuntimeError(f"synapsed did not answer on {args.sock}")

    agent = client.agent(args.agent_id, project=project)
    records = make_records(run_id, repeat=args.long_repeat)

    t0 = now_ms()
    id_by_title = {
        record.title: agent.observe(
            record.text,
            title=record.title,
            kind=record.title.split("/", 1)[0],
            tags=["agentdb", "public-bench", run_id],
            source_uri=f"bench://agentdb_public/{run_id}/{record.title}",
        )
        for record in records
    }
    if args.filler > 0:
        client.put_batch(make_noise_items(agent, args.filler), embed=False)
    ingest_ms = now_ms() - t0

    search_latencies: list[float] = []
    ranks: list[int | None] = []
    per_query: list[dict[str, Any]] = []
    for record in records:
        expected_id = id_by_title[record.title]
        for query in record.queries:
            t = now_ms()
            hits = agent.search_index(query, limit=args.top_k)
            search_latencies.append(now_ms() - t)
            rank = rank_of(hits, expected_id)
            ranks.append(rank)
            per_query.append(
                {
                    "query": query,
                    "expected_title": record.title,
                    "expected_id": expected_id,
                    "rank": rank,
                    "hit_ids": [hit["id"] for hit in hits],
                }
            )

    search_metrics = metrics_from_ranks(ranks)
    search_metrics.update(
        {
            "p50_ms": round(statistics.median(search_latencies), 3),
            "p95_ms": round(pct(search_latencies, 95), 3),
            "max_ms": round(max(search_latencies), 3),
        }
    )

    arms = []
    for arm in ARMS:
        arm_lat: list[float] = []
        arm_ranks: list[int | None] = []
        savings: list[float] = []
        hydrated_counts: list[int] = []
        token_counts: list[int] = []
        for _ in range(args.repeat):
            for record in records:
                expected_id = id_by_title[record.title]
                for query in record.queries:
                    t = now_ms()
                    pack = agent.context_pack(
                        query,
                        token_budget=arm.token_budget,
                        index_k=arm.index_k,
                        full_k=arm.full_k,
                        snippet_chars=arm.snippet_chars,
                    )
                    arm_lat.append(now_ms() - t)
                    arm_ranks.append(rank_of(pack["index"], expected_id))
                    savings.append(float(pack["token_savings_pct"]))
                    hydrated_counts.append(len(pack["observations"]))
                    token_counts.append(int(pack["estimated_tokens"]))

        m = metrics_from_ranks(arm_ranks)
        p95_ms = pct(arm_lat, 95)
        token_savings = statistics.mean(savings) if savings else 0.0
        score = (
            m["r_at_5"] * 100.0
            + m["mrr"] * 20.0
            + token_savings * 0.20
            - p95_ms * 0.50
        )
        arms.append(
            {
                "name": arm.name,
                "config": {
                    "token_budget": arm.token_budget,
                    "index_k": arm.index_k,
                    "full_k": arm.full_k,
                    "snippet_chars": arm.snippet_chars,
                },
                **m,
                "p50_ms": round(statistics.median(arm_lat), 3),
                "p95_ms": round(p95_ms, 3),
                "max_ms": round(max(arm_lat), 3),
                "token_savings_pct": round(token_savings, 1),
                "avg_hydrated": round(statistics.mean(hydrated_counts), 2),
                "avg_estimated_tokens": round(statistics.mean(token_counts), 1),
                "score": round(score, 3),
            }
        )
    arms.sort(key=lambda row: row["score"], reverse=True)

    return {
        "run_id": run_id,
        "project": project,
        "agent_id": args.agent_id,
        "stored_docs": len(records) + max(args.filler, 0),
        "target_docs": len(records),
        "filler_docs": max(args.filler, 0),
        "queries": len(per_query),
        "top_k": args.top_k,
        "ingest_ms": round(ingest_ms, 3),
        "search_index": search_metrics,
        "arms": arms,
        "winner": arms[0],
        "per_query": per_query,
    }


def render_markdown(result: dict[str, Any]) -> str:
    lines = [
        "# Synapse AgentDB Public Benchmark",
        "",
        f"Run: `{result['run_id']}`",
        f"Project: `{result['project']}`",
        (
            f"Stored docs: `{result['stored_docs']}` "
            f"(targets `{result['target_docs']}`, filler `{result['filler_docs']}`); "
            f"queries: `{result['queries']}`; top-k: `{result['top_k']}`"
        ),
        f"Ingest: `{result['ingest_ms']} ms`",
        "",
        "## Search Index",
        "",
        "| R@1 | R@5 | MRR | p50 ms | p95 ms | max ms |",
        "|---:|---:|---:|---:|---:|---:|",
    ]
    s = result["search_index"]
    lines.append(
        f"| {s['r_at_1']} | {s['r_at_5']} | {s['mrr']} | {s['p50_ms']} | {s['p95_ms']} | {s['max_ms']} |"
    )
    lines.extend(
        [
            "",
            "## Context-Pack Race",
            "",
            "| Rank | Arm | R@5 | MRR | p95 ms | Token savings | Avg hydrated | Score |",
            "|---:|---|---:|---:|---:|---:|---:|---:|",
        ]
    )
    for idx, arm in enumerate(result["arms"], 1):
        lines.append(
            f"| {idx} | `{arm['name']}` | {arm['r_at_5']} | {arm['mrr']} | "
            f"{arm['p95_ms']} | {arm['token_savings_pct']}% | {arm['avg_hydrated']} | {arm['score']} |"
        )
    winner = result["winner"]
    lines.extend(
        [
            "",
            "## Winner",
            "",
            f"`{winner['name']}` with config `{json.dumps(winner['config'], sort_keys=True)}`.",
            "",
            "Score is a local tuning objective: `R@5*100 + MRR*20 + token_savings*0.2 - p95_ms*0.5`.",
            "It is useful for regression and config selection, not a public SOTA recall claim by itself.",
        ]
    )
    return "\n".join(lines) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--sock", default="/tmp/synapse.sock")
    parser.add_argument("--agent-id", default="publicbench")
    parser.add_argument("--project")
    parser.add_argument("--run-id")
    parser.add_argument("--top-k", type=int, default=5)
    parser.add_argument("--repeat", type=int, default=3)
    parser.add_argument("--long-repeat", type=int, default=12)
    parser.add_argument("--filler", type=int, default=96)
    parser.add_argument("--no-write-results", action="store_true")
    args = parser.parse_args()

    result = run_benchmark(args)
    if not args.no_write_results:
        RESULTS.mkdir(parents=True, exist_ok=True)
        stem = f"agentdb_public_{time.strftime('%Y%m%d-%H%M%S')}"
        (RESULTS / f"{stem}.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
        (RESULTS / f"{stem}.md").write_text(render_markdown(result), encoding="utf-8")
        result["result_json"] = str(RESULTS / f"{stem}.json")
        result["result_md"] = str(RESULTS / f"{stem}.md")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
