#!/usr/bin/env python3
"""dashboard.py — auto-generate HTML dashboard from bench results."""

import json
import os
import sys
from pathlib import Path
from collections import defaultdict

DIR = Path(__file__).parent
RESULTS_DIR = DIR / "results"
OUT_HTML = DIR / "dashboard.html"

# ── Load all JSONL results ────────────────────────────────────────────────────

def load_results():
    rows = []
    for f in sorted(RESULTS_DIR.glob("*.jsonl")):
        if f.name.startswith("summary"):
            continue
        engine = f.stem.rsplit("_", 1)[0]
        profile = f.stem.rsplit("_", 1)[1]
        with open(f) as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                try:
                    d = json.loads(line)
                    d["_engine"] = engine
                    d["_profile"] = profile
                    rows.append(d)
                except json.JSONDecodeError:
                    pass
    return rows

# ── Chart data extraction ─────────────────────────────────────────────────────

def extract_phase_ops(rows):
    """Bar chart: ops/sec per engine × phase (A/B/C/D)."""
    data = defaultdict(dict)
    for r in rows:
        eng = r.get("engine", r["_engine"])
        for ph in ["phase_a", "phase_b", "phase_c", "phase_d"]:
            if ph in r:
                ops = r[ph].get("ops_sec", 0)
                data[eng][ph.upper().replace("PHASE_", "Phase ")] = ops
    return data

def extract_concurrency(rows):
    """Line chart: Phase F concurrency saturation (workers vs ops/sec)."""
    data = defaultdict(list)
    for r in rows:
        eng = r.get("engine", r["_engine"])
        pf = r.get("phase_f", {})
        if pf and "curve" in pf:
            for pt in pf["curve"]:
                data[eng].append((pt.get("workers", 0), pt.get("ops_sec", 0)))
        elif pf and "ops_sec" in pf:
            workers = pf.get("threads", pf.get("workers", 1))
            data[eng].append((workers, pf["ops_sec"]))
    return data

def extract_heatmap(rows):
    """Heatmap: Phase H batch-size × engine → ops/sec."""
    # {engine: {batch_size: ops_sec}}
    data = defaultdict(dict)
    for r in rows:
        eng = r.get("engine", r["_engine"])
        ph = r.get("phase_h", {})
        if ph:
            if "batch_results" in ph:
                for br in ph["batch_results"]:
                    bs = str(br.get("batch_size", "?"))
                    data[eng][bs] = br.get("ops_sec", 0)
            elif "ops_sec" in ph:
                bs = str(ph.get("batch_size", "1"))
                data[eng][bs] = ph["ops_sec"]
    return data

def extract_pareto(rows):
    """Scatter: throughput vs recall@10."""
    pts = []
    for r in rows:
        eng = r.get("engine", r["_engine"])
        pa = r.get("phase_a", {})
        pe = r.get("phase_e", {})
        if pa and pe:
            pts.append({
                "engine": eng,
                "ops_sec": pa.get("ops_sec", 0),
                "recall_at_10": pe.get("recall_at_10", 0),
            })
    return pts

def extract_latency(rows):
    """Latency CDF: p50/p95/p99 per engine."""
    data = {}
    for r in rows:
        eng = r.get("engine", r["_engine"])
        for ph in ["phase_c", "phase_d"]:
            if ph in r:
                p = r[ph]
                if "p50_ms" in p:
                    data[eng] = {
                        "p50": p.get("p50_ms", 0),
                        "p95": p.get("p95_ms", 0),
                        "p99": p.get("p99_ms", 0),
                    }
                    break
    return data

def extract_resources(rows):
    """Resource bars: peak RSS_MB and avg CPU% per engine."""
    data = {}
    for r in rows:
        eng = r.get("engine", r["_engine"])
        rss = 0
        cpu = 0
        for ph in ["phase_a", "phase_b", "phase_c", "phase_d"]:
            p = r.get(ph, {})
            rss = max(rss, p.get("rss_mb", 0))
            cpu = max(cpu, p.get("cpu_pct_mean", 0))
        if rss or cpu:
            data[eng] = {"rss_mb": rss, "cpu_pct": cpu}
    return data

# ── HTML generation ───────────────────────────────────────────────────────────

def try_plotly():
    try:
        import plotly  # noqa
        return True
    except ImportError:
        return False

def build_plotly_html(rows):
    import plotly.graph_objects as go
    from plotly.subplots import make_subplots
    import plotly.io as pio

    phase_ops = extract_phase_ops(rows)
    concurrency = extract_concurrency(rows)
    heatmap = extract_heatmap(rows)
    pareto = extract_pareto(rows)
    latency = extract_latency(rows)
    resources = extract_resources(rows)

    figs_html = []

    # 1. Bar chart: ops/sec per engine × phase
    engines = sorted(phase_ops.keys())
    phases = ["Phase A", "Phase B", "Phase C", "Phase D"]
    fig1 = go.Figure()
    for ph in phases:
        y = [phase_ops[e].get(ph, 0) for e in engines]
        fig1.add_trace(go.Bar(name=ph, x=engines, y=y))
    fig1.update_layout(
        title="Ops/sec per Engine × Phase",
        xaxis_title="Engine", yaxis_title="Ops/sec",
        barmode="group", template="plotly_dark",
        legend_title="Phase"
    )
    figs_html.append(("<h2>Phase Throughput (A/B/C/D)</h2>", pio.to_html(fig1, include_plotlyjs=False, full_html=False)))

    # 2. Line chart: Phase F concurrency
    fig2 = go.Figure()
    has_f = False
    for eng, pts in sorted(concurrency.items()):
        if pts:
            pts_sorted = sorted(pts)
            fig2.add_trace(go.Scatter(
                x=[p[0] for p in pts_sorted],
                y=[p[1] for p in pts_sorted],
                mode="lines+markers", name=eng
            ))
            has_f = True
    if not has_f:
        # Fallback: use phase_d as single point
        for r in rows:
            eng = r.get("engine", r["_engine"])
            pd = r.get("phase_d", {})
            if pd and "ops_sec" in pd:
                fig2.add_trace(go.Scatter(
                    x=[pd.get("threads", 1)],
                    y=[pd["ops_sec"]],
                    mode="markers+text", name=eng,
                    text=[eng], textposition="top center"
                ))
    fig2.update_layout(
        title="Phase F: Concurrency Saturation",
        xaxis_title="Workers", yaxis_title="Ops/sec",
        template="plotly_dark"
    )
    figs_html.append(("<h2>Concurrency Saturation (Phase F)</h2>", pio.to_html(fig2, include_plotlyjs=False, full_html=False)))

    # 3. Heatmap: batch-size × engine
    if heatmap:
        all_bs = sorted(set(bs for e in heatmap.values() for bs in e), key=lambda x: int(x) if x.isdigit() else 0)
        z = [[heatmap[e].get(bs, 0) for bs in all_bs] for e in sorted(heatmap.keys())]
        fig3 = go.Figure(go.Heatmap(
            z=z, x=all_bs, y=sorted(heatmap.keys()),
            colorscale="Viridis", colorbar_title="Ops/sec",
            hoverongaps=False
        ))
        fig3.update_layout(
            title="Phase H: Batch Size × Engine Heatmap",
            xaxis_title="Batch Size", yaxis_title="Engine",
            template="plotly_dark"
        )
        figs_html.append(("<h2>Batch Throughput Heatmap (Phase H)</h2>", pio.to_html(fig3, include_plotlyjs=False, full_html=False)))

    # 4. Pareto scatter
    if pareto:
        fig4 = go.Figure()
        for pt in pareto:
            fig4.add_trace(go.Scatter(
                x=[pt["ops_sec"]], y=[pt["recall_at_10"]],
                mode="markers+text", name=pt["engine"],
                text=[pt["engine"]], textposition="top center",
                marker=dict(size=14)
            ))
        fig4.update_layout(
            title="Throughput vs Recall@10 Pareto Frontier",
            xaxis_title="Ops/sec (higher=faster)",
            yaxis_title="Recall@10 (higher=better)",
            template="plotly_dark"
        )
        figs_html.append(("<h2>Pareto Frontier: Throughput vs Recall</h2>", pio.to_html(fig4, include_plotlyjs=False, full_html=False)))

    # 5. Latency CDF
    if latency:
        fig5 = go.Figure()
        for eng, lats in sorted(latency.items()):
            fig5.add_trace(go.Bar(
                name=eng,
                x=["p50", "p95", "p99"],
                y=[lats["p50"], lats["p95"], lats["p99"]],
                text=[f"{v:.2f}ms" for v in [lats["p50"], lats["p95"], lats["p99"]]],
                textposition="auto"
            ))
        fig5.update_layout(
            title="Latency Distribution (p50/p95/p99, log scale)",
            xaxis_title="Percentile", yaxis_title="Latency (ms)",
            yaxis_type="log", barmode="group",
            template="plotly_dark"
        )
        figs_html.append(("<h2>Latency CDF per Engine</h2>", pio.to_html(fig5, include_plotlyjs=False, full_html=False)))

    # 6. Resource bars
    if resources:
        engs = sorted(resources.keys())
        fig6 = go.Figure()
        fig6.add_trace(go.Bar(name="Peak RSS (MB)", x=engs,
                              y=[resources[e]["rss_mb"] for e in engs], yaxis="y"))
        fig6.add_trace(go.Bar(name="Avg CPU%", x=engs,
                              y=[resources[e]["cpu_pct"] for e in engs], yaxis="y2"))
        fig6.update_layout(
            title="Resource Usage per Engine",
            xaxis_title="Engine",
            yaxis=dict(title="Peak RSS (MB)", side="left"),
            yaxis2=dict(title="Avg CPU%", overlaying="y", side="right"),
            barmode="group", template="plotly_dark"
        )
        figs_html.append(("<h2>Resource Usage (RAM + CPU)</h2>", pio.to_html(fig6, include_plotlyjs=False, full_html=False)))

    # Assemble HTML
    plotly_cdn = '<script src="https://cdn.plot.ly/plotly-2.27.0.min.js"></script>'
    body = "\n".join(f"{title}\n{div}" for title, div in figs_html)
    return f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<title>Synapse Bench Dashboard</title>
{plotly_cdn}
<style>
  body {{ background: #1a1a2e; color: #eee; font-family: sans-serif; margin: 20px; }}
  h1 {{ color: #a78bfa; }}
  h2 {{ color: #7dd3fc; border-bottom: 1px solid #334; padding-bottom: 4px; }}
  .chart {{ margin-bottom: 40px; }}
</style>
</head>
<body>
<h1>Synapse Comprehensive Bench — Dashboard</h1>
<p>Generated: {__import__('datetime').datetime.now().isoformat()}</p>
{body}
</body>
</html>"""

def build_matplotlib_html(rows):
    """Fallback: embed PNG charts as base64."""
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    import base64
    from io import BytesIO

    def fig_to_b64(fig):
        buf = BytesIO()
        fig.savefig(buf, format="png", bbox_inches="tight", facecolor="#1a1a2e")
        buf.seek(0)
        return base64.b64encode(buf.read()).decode()

    charts_html = []
    plt.style.use("dark_background")

    phase_ops = extract_phase_ops(rows)
    engines = sorted(phase_ops.keys())
    phases = ["Phase A", "Phase B", "Phase C", "Phase D"]

    if phase_ops:
        fig, ax = plt.subplots(figsize=(10, 5))
        x = range(len(engines))
        width = 0.2
        for i, ph in enumerate(phases):
            vals = [phase_ops[e].get(ph, 0) for e in engines]
            ax.bar([xi + i * width for xi in x], vals, width, label=ph)
        ax.set_xticks([xi + width for xi in x])
        ax.set_xticklabels(engines)
        ax.set_title("Ops/sec per Engine × Phase")
        ax.set_ylabel("Ops/sec")
        ax.legend()
        charts_html.append(f'<h2>Phase Throughput</h2><img src="data:image/png;base64,{fig_to_b64(fig)}" style="max-width:100%">')
        plt.close(fig)

    latency = extract_latency(rows)
    if latency:
        fig, ax = plt.subplots(figsize=(10, 4))
        engs = sorted(latency.keys())
        for i, pct in enumerate(["p50", "p95", "p99"]):
            vals = [latency[e][pct] for e in engs]
            ax.bar([xi + i * 0.25 for xi in range(len(engs))], vals, 0.25, label=pct)
        ax.set_yscale("log")
        ax.set_xticks(range(len(engs)))
        ax.set_xticklabels(engs)
        ax.set_title("Latency (ms, log scale)")
        ax.legend()
        charts_html.append(f'<h2>Latency CDF</h2><img src="data:image/png;base64,{fig_to_b64(fig)}" style="max-width:100%">')
        plt.close(fig)

    resources = extract_resources(rows)
    if resources:
        fig, ax = plt.subplots(figsize=(10, 4))
        engs = sorted(resources.keys())
        ax.bar(engs, [resources[e]["rss_mb"] for e in engs])
        ax.set_title("Peak RSS MB per Engine")
        ax.set_ylabel("MB")
        charts_html.append(f'<h2>Memory Usage</h2><img src="data:image/png;base64,{fig_to_b64(fig)}" style="max-width:100%">')
        plt.close(fig)

    body = "\n".join(charts_html)
    return f"""<!DOCTYPE html>
<html lang="en">
<head><meta charset="UTF-8"><title>Synapse Bench Dashboard (matplotlib)</title>
<style>body{{background:#1a1a2e;color:#eee;font-family:sans-serif;margin:20px}}h1{{color:#a78bfa}}h2{{color:#7dd3fc}}</style>
</head>
<body>
<h1>Synapse Bench Dashboard (matplotlib fallback)</h1>
{body}
</body></html>"""

def main():
    rows = load_results()
    if not rows:
        print("[dashboard] No results found in results/", file=sys.stderr)
        sys.exit(1)
    print(f"[dashboard] Loaded {len(rows)} result rows from {RESULTS_DIR}")

    if try_plotly():
        print("[dashboard] Using Plotly")
        html = build_plotly_html(rows)
    else:
        print("[dashboard] Plotly not found, using matplotlib fallback")
        html = build_matplotlib_html(rows)

    OUT_HTML.write_text(html)
    print(f"[dashboard] Written: {OUT_HTML}")
    print(f"[dashboard] Open: open {OUT_HTML}")

if __name__ == "__main__":
    main()
