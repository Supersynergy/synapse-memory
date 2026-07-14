#!/usr/bin/env python3
"""Measure the user-visible portable Synapse Memory path with stdlib only."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import shutil
import subprocess
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


def args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin", type=Path, required=True)
    parser.add_argument("--iterations", type=int, default=100)
    parser.add_argument("--json-out", type=Path)
    parser.add_argument("--markdown-out", type=Path)
    return parser.parse_args()


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    index = max(0, math.ceil(len(ordered) * fraction) - 1)
    return ordered[index]


def timed(command: list[str], env: dict[str, str]) -> tuple[float, subprocess.CompletedProcess[str]]:
    start = time.perf_counter_ns()
    result = subprocess.run(command, env=env, text=True, capture_output=True, check=True)
    elapsed_ms = (time.perf_counter_ns() - start) / 1_000_000
    return elapsed_ms, result


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def peak_rss_bytes(command: list[str], env: dict[str, str]) -> int | None:
    time_bin = Path("/usr/bin/time")
    if not time_bin.exists():
        return None
    flag = "-l" if sys_platform() == "darwin" else "-v"
    result = subprocess.run(
        [str(time_bin), flag, *command],
        env=env,
        text=True,
        capture_output=True,
        check=True,
    )
    for line in result.stderr.splitlines():
        if sys_platform() == "darwin" and "maximum resident set size" in line:
            return int(line.strip().split()[0])
        if "Maximum resident set size (kbytes)" in line:
            return int(line.rsplit(":", 1)[1].strip()) * 1024
    return None


def sys_platform() -> str:
    return platform.system().lower()


def markdown(metrics: dict[str, Any]) -> str:
    return f"""# Synapse Memory local footprint

Generated `{metrics['generated_at']}` on `{metrics['platform']}` with
`{metrics['version']}`. CLI process startup is included in every latency.

| Metric | Result |
|---|---:|
| Binary | {metrics['binary_bytes'] / 1_048_576:.2f} MiB |
| Local copy install | {metrics['local_copy_install_ms']:.2f} ms |
| Init | {metrics['init_ms']:.2f} ms |
| Remember p50 / p95 | {metrics['remember_p50_ms']:.2f} / {metrics['remember_p95_ms']:.2f} ms |
| First cited context | {metrics['first_context_ms']:.2f} ms |
| Warm context p50 / p95 | {metrics['context_p50_ms']:.2f} / {metrics['context_p95_ms']:.2f} ms |
| Peak RSS, one context | {metrics['context_peak_rss_bytes'] / 1_048_576:.2f} MiB |
| SQLite bytes after {metrics['records']} records | {metrics['database_bytes']} B |

Binary SHA-256: `{metrics['binary_sha256']}`.

Scope: local lexical portable build. No model, provider, network, daemon, or
competitor service. This is footprint evidence, not a cross-product recall claim.
"""


def main() -> int:
    options = args()
    binary = options.bin.expanduser().resolve(strict=True)
    if options.iterations < 20:
        raise SystemExit("--iterations must be at least 20")

    with tempfile.TemporaryDirectory(prefix="synapse-memory-bench.") as raw_tmp:
        tmp = Path(raw_tmp)
        home = tmp / "home"
        prefix = home / ".local/bin"
        prefix.mkdir(parents=True)
        env = os.environ.copy()
        env["HOME"] = str(home)
        installed = prefix / ("synx.exe" if binary.suffix == ".exe" else "synx")

        start = time.perf_counter_ns()
        shutil.copy2(binary, installed)
        installed.chmod(0o755)
        local_copy_install_ms = (time.perf_counter_ns() - start) / 1_000_000

        version = subprocess.run(
            [str(installed), "--version"], env=env, text=True, capture_output=True, check=True
        ).stdout.strip()
        db = home / ".synapse/brain.db"
        base = [str(installed), "-f", str(db)]
        init_ms, _ = timed([*base, "init"], env)

        remember_ms: list[float] = []
        for index in range(options.iterations):
            elapsed, _ = timed(
                [
                    *base,
                    "remember",
                    "--kind",
                    "decision",
                    f"Release record {index}: verify checksum before install and preserve memory on uninstall.",
                ],
                env,
            )
            remember_ms.append(elapsed)

        query = [*base, "context", "checksum install preserve memory", "--mode", "coding", "--json"]
        first_context_ms, first = timed(query, env)
        context_payload = json.loads(first.stdout)
        if not context_payload.get("context_id") or context_payload.get("route") != "lexical":
            raise SystemExit("context correctness gate failed")

        context_ms = [timed(query, env)[0] for _ in range(options.iterations)]
        rss = peak_rss_bytes(query, env)
        if rss is None:
            raise SystemExit("could not measure peak RSS with /usr/bin/time")

        database_bytes = sum(
            path.stat().st_size
            for path in (db, Path(f"{db}-wal"), Path(f"{db}-shm"))
            if path.exists()
        )
        metrics: dict[str, Any] = {
            "schema": 1,
            "generated_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
            "platform": f"{platform.system()} {platform.release()} {platform.machine()}",
            "version": version,
            "records": options.iterations,
            "binary_bytes": binary.stat().st_size,
            "binary_sha256": sha256(binary),
            "local_copy_install_ms": round(local_copy_install_ms, 3),
            "init_ms": round(init_ms, 3),
            "remember_p50_ms": round(percentile(remember_ms, 0.50), 3),
            "remember_p95_ms": round(percentile(remember_ms, 0.95), 3),
            "first_context_ms": round(first_context_ms, 3),
            "context_p50_ms": round(percentile(context_ms, 0.50), 3),
            "context_p95_ms": round(percentile(context_ms, 0.95), 3),
            "context_peak_rss_bytes": rss,
            "database_bytes": database_bytes,
        }

        rendered_json = json.dumps(metrics, indent=2) + "\n"
        if options.json_out:
            options.json_out.parent.mkdir(parents=True, exist_ok=True)
            options.json_out.write_text(rendered_json)
        if options.markdown_out:
            options.markdown_out.parent.mkdir(parents=True, exist_ok=True)
            options.markdown_out.write_text(markdown(metrics))
        print(rendered_json, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
