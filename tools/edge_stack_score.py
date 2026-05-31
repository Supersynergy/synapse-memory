#!/usr/bin/env python3
"""Rank edge-stack building blocks by leverage.

This is intentionally deterministic today. It gives the autolearn loop a stable
reward ledger shape later: replace or update the input scores with measured
latency, recall, ingestion throughput, and user outcome deltas.
"""

from __future__ import annotations

import argparse
import json
from dataclasses import asdict, dataclass


WEIGHTS = {
    "speed": 0.18,
    "compounding": 0.20,
    "moat": 0.18,
    "fit": 0.16,
    "maturity": 0.12,
    "optionality": 0.10,
    "complexity": -0.04,
    "risk": -0.02,
}


@dataclass(frozen=True)
class Candidate:
    name: str
    role: str
    pattern: str
    speed: int
    compounding: int
    moat: int
    fit: int
    maturity: int
    optionality: int
    complexity: int
    risk: int

    @property
    def score(self) -> float:
        return sum(getattr(self, key) * weight for key, weight in WEIGHTS.items())


CANDIDATES = [
    Candidate(
        "CocoIndex-style CDC flow",
        "incremental source plane",
        "flow_def + FlowBuilder/DataScope + stable target names + max_inflight budgets",
        8,
        10,
        9,
        9,
        7,
        9,
        5,
        4,
    ),
    Candidate(
        "Synapse batch recall",
        "local memory hot path",
        "one process + one socket + N requests + portfolio context packing",
        10,
        10,
        9,
        10,
        8,
        8,
        2,
        2,
    ),
    Candidate(
        "OMEGA Memory pattern mine",
        "local-first MCP memory reference",
        "SQLite/sqlite-vec + ONNX embeddings + MCP tools + contradiction/forgetting ideas; installed isolated in .tools/omega-memory",
        7,
        8,
        7,
        8,
        6,
        8,
        4,
        4,
    ),
    Candidate(
        "LeanCTX context compression",
        "token saver and context runtime",
        "AST read modes + shell/file compression + cache-aware context governance before LLM injection",
        9,
        8,
        7,
        8,
        7,
        8,
        4,
        4,
    ),
    Candidate(
        "Context7 fallback mesh",
        "fresh documentation plane",
        "Synapse local resolved-version guard first; Context/Docfork/GitMCP/DeepWiki as remote docs and repo evidence fallback",
        7,
        9,
        7,
        9,
        7,
        9,
        5,
        4,
    ),
    Candidate(
        "Tantivy lexical tier",
        "embedded BM25/search",
        "in-process index, cached reader, memory writer budget, segment warm start",
        9,
        8,
        8,
        9,
        9,
        7,
        4,
        3,
    ),
    Candidate(
        "Qdrant optional external vector tier",
        "large-scale semantic tier",
        "Qdrant::from_url + timeout + compression + vector params builder",
        7,
        7,
        6,
        7,
        9,
        8,
        6,
        4,
    ),
    Candidate(
        "Pingora edge collector",
        "proxy/cache/rate-limit shell",
        "ProxyHttp callbacks: request_filter, upstream_peer, logging, cache filters",
        8,
        7,
        7,
        6,
        8,
        7,
        7,
        5,
    ),
    Candidate(
        "sonic-rs JSON hot path",
        "high-throughput API/event parse",
        "bytes to typed structs with sonic_rs::from_slice, fallback to serde_json only at edges",
        8,
        7,
        5,
        8,
        7,
        6,
        3,
        3,
    ),
    Candidate(
        "Polars/DuckDB signal analytics",
        "score/report plane",
        "LazyFrame logical plans plus DuckDB SQL snapshots for repeatable reports",
        8,
        8,
        7,
        8,
        9,
        8,
        4,
        3,
    ),
    Candidate(
        "OpenTelemetry + Vector evidence loop",
        "observability and reward logging",
        "span every agent/tool run, ship JSONL/OTLP to searchable evidence store",
        6,
        9,
        8,
        8,
        8,
        8,
        5,
        3,
    ),
    Candidate(
        "memvid/ripgrep-all media ingest",
        "cold archive adapters",
        "use as importer/source adapters, keep Synapse as recall and context brain",
        5,
        7,
        6,
        7,
        6,
        8,
        5,
        5,
    ),
]


def ranked() -> list[Candidate]:
    return sorted(CANDIDATES, key=lambda item: item.score, reverse=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    rows = ranked()
    if args.json:
        print(json.dumps([{**asdict(row), "score": round(row.score, 2)} for row in rows], indent=2))
        return 0

    print("| rank | score | component | role | pattern |")
    print("|---:|---:|---|---|---|")
    for idx, row in enumerate(rows, 1):
        print(f"| {idx} | {row.score:.2f} | {row.name} | {row.role} | {row.pattern} |")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
