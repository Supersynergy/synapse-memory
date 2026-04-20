#!/usr/bin/env python3
"""Score external repos for synapse-integration-relevance."""

import json
import math
import re
import sys
from pathlib import Path
from datetime import datetime, timezone

# ── weights ──────────────────────────────────────────────────────────────────
W_STARS = 0.2
W_AGENT_MEMORY = 0.3
W_RUST_TS = 0.1
W_HAS_SDK = 0.2
W_ACTIVE = 0.1
W_MCP = 0.1

AGENT_MEMORY_PAT = re.compile(
    r"\b(memory|RAG|vector|embedding|retrieval|knowledge.?graph|semantic.?search)\b", re.I
)
SDK_PAT = re.compile(r"\b(SDK|plugin|provider|adapter|integration|connector)\b", re.I)
MCP_PAT = re.compile(r"\b(MCP|model.?context.?protocol)\b", re.I)
EVAL_PAT = re.compile(r"\b(eval|benchmark|test|suite|metric|evals)\b", re.I)
COMPETITOR_PAT = re.compile(r"\b(memory|RAG)\b.*\b(store|storage|db|database|persist)\b|\b(store|storage|db|database|persist)\b.*\b(memory|RAG)\b", re.I)
BOILERPLATE_PAT = re.compile(r"\b(template|starter|boilerplate|scaffold|kit|skeleton)\b", re.I)

MAX_STARS = 200_000  # normalisation cap


def score_repo(r: dict) -> dict:
    name = r.get("name", "")
    desc = r.get("desc", "") or ""
    lang = (r.get("lang") or r.get("language") or "").lower()
    stars = int(r.get("stars", 0))
    age_days = r.get("age_days")  # days since last push; may be absent
    text = f"{name} {desc}"

    # stars: log-normalised 0-1
    s_stars = min(math.log1p(stars) / math.log1p(MAX_STARS), 1.0)

    # agent/memory match
    s_mem = 1.0 if AGENT_MEMORY_PAT.search(text) else 0.0

    # rust or typescript
    s_lang = 1.0 if lang in ("rust", "typescript") else 0.0

    # has SDK / plugin / provider
    s_sdk = 1.0 if SDK_PAT.search(text) else 0.0

    # active: last push < 90d  (use "push" field from md as "Age" else skip)
    push_raw = r.get("push") or r.get("last_push_days")
    if push_raw is not None:
        try:
            push_days = int(str(push_raw).replace("d", ""))
            s_active = 1.0 if push_days < 90 else 0.0
        except ValueError:
            s_active = 0.5
    else:
        s_active = 0.5  # unknown → neutral

    # MCP compat
    s_mcp = 1.0 if MCP_PAT.search(text) else 0.0

    score = (
        W_STARS * s_stars
        + W_AGENT_MEMORY * s_mem
        + W_RUST_TS * s_lang
        + W_HAS_SDK * s_sdk
        + W_ACTIVE * s_active
        + W_MCP * s_mcp
    )

    # bucket
    if score > 0.7:
        bucket = "direct-adapter"
    elif EVAL_PAT.search(text):
        bucket = "eval-tool"
    elif COMPETITOR_PAT.search(text):
        bucket = "competitor"
    elif BOILERPLATE_PAT.search(text):
        bucket = "gtm-multiplier"
    elif score < 0.3:
        bucket = "skip"
    else:
        bucket = "eval-tool" if EVAL_PAT.search(text) else "gtm-multiplier"

    return {**r, "score": round(score, 4), "bucket": bucket}


def load_md_repos(path: Path) -> list[dict]:
    """Parse the markdown table rows from agent_top*.md."""
    repos = []
    current_cluster = ""
    row_pat = re.compile(
        r"\|\s*\d+\s*\|\s*\[([^\]]+)\]\(([^)]+)\)\s*\|\s*([\d,]+)\s*\|\s*(\d+d)\s*\|\s*(\d+d)\s*\|\s*([^|]*)\s*\|\s*([\d.]+)\s*\|\s*([^|]*)\s*\|"
    )
    cluster_pat = re.compile(r"^##\s+(.+)")
    for line in path.read_text().splitlines():
        m_c = cluster_pat.match(line)
        if m_c:
            current_cluster = m_c.group(1).strip()
        m_r = row_pat.match(line)
        if m_r:
            name, url, stars_raw, age, push, lang, score_orig, desc = m_r.groups()
            repos.append({
                "name": name,
                "url": url,
                "stars": int(stars_raw.replace(",", "")),
                "age_days": int(age.replace("d", "")),
                "push": push,
                "lang": lang.strip(),
                "desc": desc.strip(),
                "cluster": current_cluster,
            })
    return repos


def load_json_repos(path: Path) -> list[dict]:
    data = json.loads(path.read_text())
    repos = []
    if isinstance(data, list):
        return data
    if "top_by_domain" in data:
        for domain, items in data["top_by_domain"].items():
            for item in items:
                repos.append({**item, "cluster": domain})
        return repos
    return repos


def render_markdown(scored: list[dict]) -> str:
    from collections import defaultdict
    BUCKET_EMOJI = {
        "direct-adapter": "🎯",
        "eval-tool": "📊",
        "competitor": "⚔️",
        "gtm-multiplier": "🔥",
        "skip": "❌",
    }
    by_bucket: dict[str, list] = defaultdict(list)
    for r in scored:
        by_bucket[r["bucket"]].append(r)

    lines = [f"# Gamechanger Synapse Report — {datetime.now(timezone.utc).date()}\n"]
    lines.append("## Stats\n")
    total = len(scored)
    lines.append(f"Total repos scored: **{total}**\n")
    lines.append("| Bucket | Count |")
    lines.append("|--------|-------|")
    for b, emoji in BUCKET_EMOJI.items():
        lines.append(f"| {emoji} {b} | {len(by_bucket[b])} |")
    lines.append("")

    for bucket, emoji in BUCKET_EMOJI.items():
        top = sorted(by_bucket[bucket], key=lambda x: x["score"], reverse=True)[:20]
        if not top:
            continue
        lines.append(f"## {emoji} {bucket}\n")
        lines.append("| # | Repo | Stars | Score | Lang | Desc |")
        lines.append("|---|------|-------|-------|------|------|")
        for i, r in enumerate(top, 1):
            name = r.get("name", "")
            url = r.get("url", "")
            link = f"[{name}]({url})" if url else name
            stars = r.get("stars", 0)
            score = r["score"]
            lang = r.get("lang", r.get("language", ""))
            desc = (r.get("desc") or "")[:80]
            lines.append(f"| {i} | {link} | {stars:,} | {score:.3f} | {lang} | {desc} |")
        lines.append("")

    return "\n".join(lines)


def main():
    # locate input
    candidates = [
        Path("/Users/master/projects/docs/awesome-indexer/agent_top1000_FRESH_20260419.md"),
        Path("/Users/master/projects/docs/awesome-indexer/agent_top1000_20260419.md"),
        Path("/Users/master/projects/skill-library/top1000_report.json"),
    ]
    repos = []
    used = None
    for c in candidates:
        if c.exists():
            used = c
            if c.suffix == ".md":
                repos = load_md_repos(c)
            else:
                repos = load_json_repos(c)
            if repos:
                break

    if not repos and not sys.stdin.isatty():
        repos = json.load(sys.stdin)

    if not repos:
        print("No input found. Pipe JSON or ensure top-list files exist.", file=sys.stderr)
        sys.exit(1)

    print(f"Loaded {len(repos)} repos from {used}", file=sys.stderr)

    scored = [score_repo(r) for r in repos]
    scored.sort(key=lambda x: x["score"], reverse=True)

    # write outputs
    out_dir = Path("/Users/master/projects/synapse/data")
    out_dir.mkdir(exist_ok=True)

    json_out = out_dir / "gamechanger_scores.json"
    json_out.write_text(json.dumps(scored, indent=2, ensure_ascii=False))
    print(f"JSON: {json_out}", file=sys.stderr)

    md_out = out_dir / "gamechanger_report.md"
    md_out.write_text(render_markdown(scored))
    print(f"Report: {md_out}", file=sys.stderr)

    # print top-5 per bucket to stdout
    from collections import defaultdict
    by_bucket: dict[str, list] = defaultdict(list)
    for r in scored:
        by_bucket[r["bucket"]].append(r)
    for bucket in ("direct-adapter", "eval-tool", "competitor", "gtm-multiplier"):
        top5 = sorted(by_bucket[bucket], key=lambda x: x["score"], reverse=True)[:5]
        print(f"\n{bucket}:")
        for r in top5:
            print(f"  {r['score']:.3f}  {r['name']}")


if __name__ == "__main__":
    main()
