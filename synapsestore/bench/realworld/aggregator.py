"""Bench-result aggregator — emits bench_results.json + bench_results.html.

Each real-world harness above prints human-readable numbers. The aggregator
shells out to every harness, scrapes the `p50 / p95 / p99` line + the
"consumer framing" line, and writes one structured record per scenario.

Usage:
    # any env var set for bench_all.sh also flows through here
    python aggregator.py
"""

from __future__ import annotations
import json
import os
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCENARIOS = [
    # (id, script, env_var)
    ("a01", "01_obsidian.py",       "OBSIDIAN_VAULT"),
    ("a03", "03_apple_notes.py",    "NOTES_DB"),
    ("a04", "04_logseq.py",         "LOGSEQ"),
    ("b11", "11_chatgpt_history.py","CHATGPT_JSON"),
    ("c21", "21_gmail_mbox.py",     "GMAIL_MBOX"),
    ("c22", "22_slack_export.py",   "SLACK_EXPORT"),
    ("e33", "33_photo_clip.py",     None),                # synthetic
    ("h49", "49_health_fuse.py",    None),                # synthetic
]

PCT_RE     = re.compile(r"p50 */ *p95 */ *p99\s+(\d+)\s*/\s*(\d+)\s*/\s*(\d+)")
FRAMING_RE = re.compile(r"consumer framing:\s*«(.+)»")


def run_one(sid: str, script: str, env_var: str | None) -> dict | None:
    args = [sys.executable, str(HERE / script)]
    if env_var:
        path = os.environ.get(env_var)
        if not path:
            return None
        args.append(path)
    try:
        proc = subprocess.run(args, cwd=HERE, capture_output=True, text=True, timeout=300)
    except Exception as e:
        return {"id": sid, "error": repr(e)}
    out = proc.stdout
    m = PCT_RE.search(out)
    f = FRAMING_RE.search(out)
    return {
        "id": sid,
        "script": script,
        "p50_us": int(m.group(1)) if m else None,
        "p95_us": int(m.group(2)) if m else None,
        "p99_us": int(m.group(3)) if m else None,
        "framing": f.group(1) if f else None,
        "stdout_tail": out.strip().splitlines()[-10:] if out else [],
        "returncode": proc.returncode,
    }


def write_html(records: list[dict], path: Path) -> None:
    rows = "\n".join(
        f"<tr><td>{r['id']}</td><td>{r.get('script','')}</td>"
        f"<td>{r.get('p50_us') or '—'}</td><td>{r.get('p95_us') or '—'}</td>"
        f"<td>{r.get('p99_us') or '—'}</td><td>{r.get('framing') or r.get('error','')}</td></tr>"
        for r in records
    )
    html = f"""<!doctype html>
<meta charset="utf-8">
<title>Synapse · real-world bench dashboard</title>
<style>
body {{ font-family: -apple-system, ui-sans-serif, system-ui; margin: 2rem; }}
table {{ border-collapse: collapse; width: 100%; }}
th,td {{ border-bottom: 1px solid #ddd; padding: .4rem .6rem; text-align: left; }}
th {{ background: #fafafa; }}
td:nth-child(3), td:nth-child(4), td:nth-child(5) {{ text-align: right; font-variant-numeric: tabular-nums; }}
</style>
<h1>Synapse · real-world bench dashboard</h1>
<p>Generated via <code>bench/realworld/aggregator.py</code>.</p>
<table>
  <thead><tr><th>id</th><th>script</th><th>p50 µs</th><th>p95 µs</th><th>p99 µs</th><th>consumer framing</th></tr></thead>
  <tbody>
    {rows}
  </tbody>
</table>
"""
    path.write_text(html, encoding="utf-8")


def main() -> int:
    records: list[dict] = []
    for sid, script, env in SCENARIOS:
        r = run_one(sid, script, env)
        if r is None:
            print(f"skip {sid}: env var {env} not set")
            continue
        records.append(r)
        print(f"ran  {sid}: p95={r.get('p95_us')} µs · {r.get('framing')}")
    out_json = HERE / "bench_results.json"
    out_html = HERE / "bench_results.html"
    out_json.write_text(json.dumps(records, indent=2), encoding="utf-8")
    write_html(records, out_html)
    print()
    print(f"▸ json   {out_json}")
    print(f"▸ html   {out_html}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
