#!/usr/bin/env python3
"""powermetrics_wrap.py — M4 Max power+thermal monitoring wrapper for bench.py.

Usage:
  python3 powermetrics_wrap.py --engine synapse --profile fast
  python3 powermetrics_wrap.py --engine synapse --profile fast --dry-run
  python3 powermetrics_wrap.py --help

Adds power_avg_w, power_peak_w, thermal_pressure_max, ops_per_watt to JSONL.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

DIR = Path(__file__).parent
RESULTS_DIR = DIR / "results"
VENV_PY = Path.home() / ".venvs/synapse-bench/bin/python3"
BENCH_PY = DIR / "bench.py"

# ── powermetrics parsing ──────────────────────────────────────────────────────

POWER_RE = re.compile(r"CPU Power:\s+([\d.]+)\s*mW", re.IGNORECASE)
THERMAL_RE = re.compile(r"thermal pressure[:\s]+([\w]+)", re.IGNORECASE)
FREQ_P_RE = re.compile(r"P-Cluster Frequency:\s+([\d.]+)\s*MHz", re.IGNORECASE)
FREQ_E_RE = re.compile(r"E-Cluster Frequency:\s+([\d.]+)\s*MHz", re.IGNORECASE)

THERMAL_LEVELS = {"nominal": 0, "light": 1, "moderate": 2, "heavy": 3, "critical": 4}


def parse_powermetrics_output(text: str) -> dict:
    """Parse raw powermetrics stdout into aggregated metrics."""
    samples_power = []
    thermal_max = 0
    freqs_p = []
    freqs_e = []

    for line in text.splitlines():
        m = POWER_RE.search(line)
        if m:
            samples_power.append(float(m.group(1)) / 1000.0)  # mW → W
        m = THERMAL_RE.search(line)
        if m:
            lvl = THERMAL_LEVELS.get(m.group(1).lower(), 0)
            thermal_max = max(thermal_max, lvl)
        m = FREQ_P_RE.search(line)
        if m:
            freqs_p.append(float(m.group(1)))
        m = FREQ_E_RE.search(line)
        if m:
            freqs_e.append(float(m.group(1)))

    result = {
        "power_avg_w": round(sum(samples_power) / len(samples_power), 2) if samples_power else None,
        "power_peak_w": round(max(samples_power), 2) if samples_power else None,
        "thermal_pressure_max": list(THERMAL_LEVELS.keys())[thermal_max] if samples_power else None,
        "p_cluster_freq_mhz_avg": round(sum(freqs_p) / len(freqs_p), 1) if freqs_p else None,
        "e_cluster_freq_mhz_avg": round(sum(freqs_e) / len(freqs_e), 1) if freqs_e else None,
    }
    return result


def psutil_fallback(duration_s: float) -> dict:
    """Fallback when powermetrics unavailable (no sudo)."""
    try:
        import psutil
        samples_freq = []
        samples_cpu = []
        t0 = time.time()
        while time.time() - t0 < duration_s:
            freq = psutil.cpu_freq()
            if freq:
                samples_freq.append(freq.current)
            samples_cpu.append(psutil.cpu_percent(interval=None))
            time.sleep(1.0)
        print("[powermetrics] WARNING: Using psutil fallback — no power data (sudo required)", file=sys.stderr)
        return {
            "power_avg_w": None,
            "power_peak_w": None,
            "thermal_pressure_max": None,
            "cpu_freq_mhz_avg": round(sum(samples_freq) / len(samples_freq), 1) if samples_freq else None,
            "cpu_pct_avg_wrap": round(sum(samples_cpu) / len(samples_cpu), 1) if samples_cpu else None,
            "note": "psutil_fallback_no_sudo",
        }
    except ImportError:
        return {"power_avg_w": None, "power_peak_w": None, "thermal_pressure_max": None,
                "note": "psutil_not_installed"}


class PowerMonitor:
    """Background power monitor thread."""

    def __init__(self, interval_ms=1000):
        self.interval_ms = interval_ms
        self._buf = []
        self._proc = None
        self._thread = None
        self._stop = threading.Event()
        self._use_powermetrics = False

    def start(self):
        # Try powermetrics (requires sudo)
        try:
            proc = subprocess.Popen(
                ["sudo", "-n", "powermetrics",
                 "--samplers", "cpu_power,thermal",
                 "-i", str(self.interval_ms),
                 "-f", "text"],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            # Quick check — if sudo -n fails, it returns immediately
            time.sleep(0.5)
            if proc.poll() is None:
                self._proc = proc
                self._use_powermetrics = True
                print("[powermetrics] Running with sudo powermetrics", file=sys.stderr)
                self._thread = threading.Thread(target=self._reader, daemon=True)
                self._thread.start()
                return
            else:
                stderr_out = proc.stderr.read()
                print(f"[powermetrics] sudo not available without password: {stderr_out.strip()}", file=sys.stderr)
        except FileNotFoundError:
            print("[powermetrics] powermetrics binary not found", file=sys.stderr)

        # psutil fallback — collect in background
        print("[powermetrics] Falling back to psutil", file=sys.stderr)
        self._psutil_samples = {"freq": [], "cpu": []}
        self._thread = threading.Thread(target=self._psutil_reader, daemon=True)
        self._thread.start()

    def _reader(self):
        for line in self._proc.stdout:
            if self._stop.is_set():
                break
            self._buf.append(line)

    def _psutil_reader(self):
        try:
            import psutil
        except ImportError:
            return
        while not self._stop.is_set():
            freq = psutil.cpu_freq()
            if freq:
                self._psutil_samples["freq"].append(freq.current)
            self._psutil_samples["cpu"].append(psutil.cpu_percent(interval=None))
            time.sleep(self.interval_ms / 1000.0)

    def stop(self) -> dict:
        self._stop.set()
        if self._proc:
            self._proc.terminate()
            self._proc.wait()
        if self._thread:
            self._thread.join(timeout=3)

        if self._use_powermetrics:
            return parse_powermetrics_output("".join(self._buf))
        else:
            samp = getattr(self, "_psutil_samples", {})
            freqs = samp.get("freq", [])
            cpus = samp.get("cpu", [])
            return {
                "power_avg_w": None,
                "power_peak_w": None,
                "thermal_pressure_max": None,
                "cpu_freq_mhz_avg": round(sum(freqs) / len(freqs), 1) if freqs else None,
                "cpu_pct_avg_wrap": round(sum(cpus) / len(cpus), 1) if cpus else None,
                "note": "psutil_fallback_no_sudo",
            }


def inject_power_into_jsonl(jsonl_path: Path, power_data: dict, bench_time_s: float):
    """Read existing JSONL, inject power fields, rewrite."""
    if not jsonl_path.exists():
        print(f"[powermetrics] JSONL not found: {jsonl_path}", file=sys.stderr)
        return
    lines = jsonl_path.read_text().strip().splitlines()
    out = []
    for line in lines:
        try:
            d = json.loads(line)
            d.update(power_data)
            # Compute ops/watt if we have both
            if power_data.get("power_avg_w"):
                total_ops = 0
                for ph in ["phase_a", "phase_b", "phase_c", "phase_d"]:
                    p = d.get(ph, {})
                    total_ops += p.get("ops_sec", 0)
                if total_ops and power_data["power_avg_w"] > 0:
                    d["ops_per_watt"] = round(total_ops / power_data["power_avg_w"], 1)
            out.append(json.dumps(d))
        except json.JSONDecodeError:
            out.append(line)
    jsonl_path.write_text("\n".join(out) + "\n")
    print(f"[powermetrics] Injected power data into {jsonl_path}")


def build_ops_per_watt_leaderboard(results_dir: Path):
    """Print leaderboard of ops/watt from all JSONL files."""
    board = []
    for f in sorted(results_dir.glob("*.jsonl")):
        if f.name.startswith("summary"):
            continue
        with open(f) as fh:
            for line in fh:
                try:
                    d = json.loads(line.strip())
                    if "ops_per_watt" in d:
                        eng = d.get("engine", f.stem)
                        board.append((d["ops_per_watt"], eng, d.get("power_avg_w", "?")))
                except json.JSONDecodeError:
                    pass
    if not board:
        print("[powermetrics] No ops/watt data yet. Run with --with-power first.")
        return
    board.sort(reverse=True)
    print("\n=== Ops/Watt Leaderboard ===")
    print(f"{'Rank':<5} {'Engine':<20} {'Ops/Watt':<15} {'Avg W'}")
    for i, (opw, eng, avgw) in enumerate(board, 1):
        print(f"{i:<5} {eng:<20} {opw:<15.1f} {avgw}")


def main():
    ap = argparse.ArgumentParser(description="Power+thermal monitoring wrapper for bench.py")
    ap.add_argument("--engine", required=True)
    ap.add_argument("--profile", default="fast")
    ap.add_argument("--n-docs", type=int, default=None)
    ap.add_argument("--phases", default="base")
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--leaderboard", action="store_true", help="Show ops/watt leaderboard and exit")
    ap.add_argument("--interval-ms", type=int, default=1000)
    args = ap.parse_args()

    if args.leaderboard:
        build_ops_per_watt_leaderboard(RESULTS_DIR)
        return

    py = VENV_PY if VENV_PY.exists() else sys.executable
    cmd = [str(py), str(BENCH_PY), "--engine", args.engine, f"--profile={args.profile}",
           f"--phases={args.phases}"]
    if args.n_docs:
        cmd += ["--n-docs", str(args.n_docs)]
    if args.dry_run:
        cmd.append("--dry-run")

    monitor = PowerMonitor(interval_ms=args.interval_ms)
    monitor.start()

    t0 = time.time()
    print(f"[powermetrics] Running: {' '.join(cmd)}")
    ret = subprocess.call(cmd)
    elapsed = time.time() - t0

    power_data = monitor.stop()
    print(f"[powermetrics] Power data: {power_data}")

    # Inject into result JSONL
    suffix = "dry" if args.dry_run else ("fast" if args.profile == "fast" else "full")
    jsonl_path = RESULTS_DIR / f"{args.engine}_{suffix}.jsonl"
    inject_power_into_jsonl(jsonl_path, power_data, elapsed)

    build_ops_per_watt_leaderboard(RESULTS_DIR)
    sys.exit(ret)


if __name__ == "__main__":
    main()
