#!/usr/bin/env python3
"""UC58b — autolearn-style extractor race.

Races RuleExtractor vs (optional) MlxExtractor on a labeled extraction set,
scores by triple-precision (extracted relations vs ground truth) + latency,
persists per-cluster winner to a Thompson-bandit DB.

Inspired by autolearn race-mode (ASHA-halving over engines). Here engines are
extractor implementations; the bandit learns which extractor wins per cluster
of query/text type (e.g. resume, scientific, news, code).

Run:
  python3 eval/usecases/UC58b_extractor_race.py [--corpus eval/golden/extract.jsonl]
                                                 [--bandit-db .synapse/extract_bandit.db]
                                                 [--engines rule[,mlx]]

Corpus format (JSONL):
  {"id":"e1","cluster":"resume","text":"Alice works at Acme...",
   "expected_relations":[["Alice","works_at","Acme"]]}

Output: per-engine table (precision, recall, F1, mean_ms) + bandit-DB updated.
"""
from __future__ import annotations
import argparse, json, os, random, sqlite3, subprocess, time, statistics
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SYNX = os.environ.get("SYNX", "synx")

def init_bandit(path: Path):
    path.parent.mkdir(parents=True, exist_ok=True)
    c = sqlite3.connect(str(path))
    c.executescript("""
    CREATE TABLE IF NOT EXISTS extract_bandit(
      cluster TEXT, engine TEXT,
      alpha REAL DEFAULT 1.0, beta REAL DEFAULT 1.0, n INTEGER DEFAULT 0,
      PRIMARY KEY(cluster, engine)
    );
    """)
    return c

def bandit_update(c, cluster: str, engine: str, score01: float):
    a, b, n = c.execute(
        "SELECT alpha,beta,n FROM extract_bandit WHERE cluster=? AND engine=?",
        (cluster, engine)).fetchone() or (1.0, 1.0, 0)
    a += score01; b += (1.0 - score01); n += 1
    c.execute("INSERT OR REPLACE INTO extract_bandit VALUES (?,?,?,?,?)",
              (cluster, engine, a, b, n))
    c.commit()

def thompson_pick(c, cluster: str, engines: list[str]) -> str:
    rows = {e: (1.0, 1.0) for e in engines}
    for e, a, b in c.execute(
        "SELECT engine,alpha,beta FROM extract_bandit WHERE cluster=?", (cluster,)):
        if e in rows: rows[e] = (a, b)
    samples = [(random.betavariate(a, b), e) for e, (a, b) in rows.items()]
    return max(samples)[1]

def extract_rule(text: str) -> tuple[list[tuple[str, str, str]], float]:
    """RuleExtractor proxy — emits zero relations (matches Rust default)."""
    t0 = time.time()
    return [], (time.time() - t0) * 1000.0

def extract_mlx(text: str, model: str = "smollm2:360m") -> tuple[list[tuple[str, str, str]], float]:
    """Subprocess to Ollama smollm2 → JSON triples. Hard timeout 8s."""
    t0 = time.time()
    prompt = (
        "Extract subject-verb-object triples from the text below. "
        "Output strict JSON: {\"relations\":[{\"s\":\"...\",\"v\":\"...\",\"o\":\"...\"}]}. "
        "Max 6 triples. No prose.\n---\n" + text[:1500]
    )
    try:
        r = subprocess.run(["ollama", "run", model, prompt],
                           capture_output=True, text=True, timeout=8)
        out = r.stdout
        # Strip code-fences
        import re
        m = re.search(r"\{.*\}", out, re.S)
        if not m: return [], (time.time() - t0) * 1000.0
        data = json.loads(m.group(0))
        triples = [(x.get("s","").strip(), x.get("v","").strip(), x.get("o","").strip())
                   for x in data.get("relations", [])]
        return [t for t in triples if all(t)], (time.time() - t0) * 1000.0
    except Exception:
        return [], (time.time() - t0) * 1000.0

ENGINES = {"rule": extract_rule, "mlx": extract_mlx}

def f1(extracted: list, expected: list) -> tuple[float, float, float]:
    if not extracted and not expected: return 1.0, 1.0, 1.0
    if not extracted: return 0.0, 0.0, 0.0
    if not expected: return 0.0, 1.0, 0.0  # precision moot, recall 1
    e_set = {tuple(t) for t in extracted}
    g_set = {tuple(t) for t in expected}
    tp = len(e_set & g_set)
    p = tp / max(1, len(e_set))
    r = tp / max(1, len(g_set))
    f = 2 * p * r / (p + r) if (p + r) else 0.0
    return p, r, f

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--corpus", default="eval/golden/extract.jsonl")
    ap.add_argument("--bandit-db", default=".synapse/extract_bandit.db")
    ap.add_argument("--engines", default="rule")
    args = ap.parse_args()

    corpus_path = Path(args.corpus)
    if not corpus_path.is_absolute(): corpus_path = ROOT / args.corpus
    if not corpus_path.exists():
        items = [
            {"id":"e1","cluster":"resume",
             "text":"Alice works at Acme Berlin since 2024.",
             "expected_relations":[["Alice","works_at","Acme"],["Acme","located_in","Berlin"]]},
            {"id":"e2","cluster":"news",
             "text":"OpenAI released GPT-5 on 2026-04-12.",
             "expected_relations":[["OpenAI","released","GPT-5"]]},
            {"id":"e3","cluster":"science",
             "text":"HippoRAG-2 outperforms GraphRAG on MuSiQue.",
             "expected_relations":[["HippoRAG-2","outperforms","GraphRAG"]]},
        ]
    else:
        items = [json.loads(l) for l in corpus_path.read_text().splitlines() if l.strip()]

    engines = [e for e in args.engines.split(",") if e in ENGINES]
    if not engines: engines = ["rule"]
    print(f"[UC58b] races {len(items)} items × {len(engines)} engines")

    bandit_path = Path(args.bandit_db)
    if not bandit_path.is_absolute(): bandit_path = ROOT / args.bandit_db
    band = init_bandit(bandit_path)

    agg = {e: {"p":[], "r":[], "f":[], "ms":[]} for e in engines}
    for it in items:
        line = f"  {it['id']:<4} cluster={it['cluster']:<8}"
        for ename in engines:
            triples, ms = ENGINES[ename](it["text"])
            p, r, f = f1(triples, it.get("expected_relations", []))
            agg[ename]["p"].append(p); agg[ename]["r"].append(r); agg[ename]["f"].append(f)
            agg[ename]["ms"].append(ms)
            bandit_update(band, it["cluster"], ename, f)
            line += f"  {ename}: F={f:.2f} ({ms:.0f}ms)"
        print(line)

    print(f"\n[UC58b] aggregate:")
    print(f"{'engine':<8} {'P':>6} {'R':>6} {'F1':>6} {'mean_ms':>8}")
    for e, d in agg.items():
        mean = lambda xs: statistics.fmean(xs) if xs else 0.0
        print(f"{e:<8} {mean(d['p']):>6.3f} {mean(d['r']):>6.3f} {mean(d['f']):>6.3f} {mean(d['ms']):>8.1f}")

    print(f"\n[UC58b] bandit posterior per cluster:")
    for cluster in sorted({it["cluster"] for it in items}):
        winner = thompson_pick(band, cluster, engines)
        print(f"  cluster={cluster:<10} → bandit picks: {winner}")
    print("[UC58b] OK")

if __name__ == "__main__":
    main()
