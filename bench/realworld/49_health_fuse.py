"""HealthKit + journal fuse bench — h49.

Synthesizes N days × 6 metrics (sleep h, HRV, steps, mood, stress, workout
min) plus a short journal entry per day. The embedding vector concatenates
numeric-metric-delta features + HashEmbedder(journal). Queries are
"days like this one".

Usage:
    python 49_health_fuse.py --days 1825   # 5 years
"""

from __future__ import annotations
import argparse
import math
import random
import statistics
import sys
import time
from dataclasses import dataclass

import synapse

from harness import HashEmbedder


@dataclass
class Day:
    sleep_h: float
    hrv: float
    steps: int
    mood: int
    stress: int
    workout_min: int
    journal: str


JOURNAL_TEMPLATES = [
    "slept well, felt strong",
    "big client presentation, exhausted",
    "long flight, jet-lagged",
    "good morning run, clear head",
    "argued with family, bad evening",
    "quiet reading day",
    "hiked with friends, energised",
]


def synth_days(n: int, seed: int = 42) -> list[Day]:
    rng = random.Random(seed)
    days: list[Day] = []
    for _ in range(n):
        sleep = round(rng.gauss(7.0, 1.3), 1)
        days.append(Day(
            sleep_h=sleep,
            hrv=round(rng.gauss(52.0, 10.0), 1),
            steps=rng.randint(2000, 18000),
            mood=rng.randint(1, 5),
            stress=rng.randint(1, 5),
            workout_min=rng.randint(0, 90),
            journal=rng.choice(JOURNAL_TEMPLATES),
        ))
    return days


def vectorize(days: list[Day], emb: HashEmbedder) -> list[tuple[int, list[float]]]:
    """Concat z-scored metric deltas + journal embedding."""
    means = {
        "sleep_h":    sum(d.sleep_h    for d in days) / len(days),
        "hrv":        sum(d.hrv        for d in days) / len(days),
        "steps":      sum(d.steps      for d in days) / len(days),
        "workout_min":sum(d.workout_min for d in days) / len(days),
    }
    out: list[tuple[int, list[float]]] = []
    for i, d in enumerate(days):
        numeric = [
            (d.sleep_h    - means["sleep_h"])     / 1.3,
            (d.hrv        - means["hrv"])         / 10.0,
            (d.steps      - means["steps"])       / 5000.0,
            (d.mood - 3)  / 2.0,
            (d.stress - 3)/ 2.0,
            (d.workout_min - means["workout_min"]) / 30.0,
        ]
        jvec = emb._h(d.journal)  # reuse HashEmbedder's inner
        combined = numeric + jvec
        norm = math.sqrt(sum(x * x for x in combined)) or 1.0
        out.append((i, [x / norm for x in combined]))
    return out


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--days", type=int, default=1825, help="5 years default")
    ap.add_argument("--queries", type=int, default=20)
    args = ap.parse_args(argv)

    emb = HashEmbedder(dim=128)
    print(f"▸ synthesizing {args.days:,} days …")
    days = synth_days(args.days)
    rows = vectorize(days, emb)

    t0 = time.perf_counter()
    ham = synapse.HammingIndex.build(rows)
    i8 = synapse.I8Index.build(rows)
    t_build = (time.perf_counter() - t0) * 1000

    rng = random.Random(7)
    q_ids = [rng.randrange(len(rows)) for _ in range(args.queries)]
    durs = []
    for qi in q_ids:
        q = rows[qi][1]
        t = time.perf_counter()
        _ = synapse.rerank(ham, i8, q, k=10, candidates=80) if len(rows) >= 1000 else i8.search(q, k=10)
        durs.append((time.perf_counter() - t) * 1e6)
    ds = sorted(durs)
    def pct(p): return ds[min(len(ds) - 1, int(p * len(ds)))]

    print(f"▸ build indices          {t_build:>7.1f} ms")
    print(f"▸ p50 / p95 / p99        {statistics.median(durs):>5.0f} / {pct(0.95):>5.0f} / {pct(0.99):>5.0f} µs")
    print()
    print(f"consumer framing: «'find days like this one' across {args.days:,} days → {pct(0.95)/1000:.2f} ms»")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
