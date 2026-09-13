#!/usr/bin/env python3
"""Validate and optionally register official source watches in Synapse."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path


PRODUCT_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_REGISTRY = PRODUCT_ROOT / "files" / "primary-sources.json"
DEFAULT_DB = Path.home() / ".synapse" / "brain.db"


def load_registry(path: Path) -> list[dict[str, object]]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    if payload.get("schema_version") != 1:
        raise ValueError("schema_version must be 1")
    sources = payload.get("sources")
    if not isinstance(sources, list) or not sources:
        raise ValueError("sources must be a non-empty list")

    seen_ids: set[str] = set()
    seen_urls: set[str] = set()
    required = {
        "id",
        "title",
        "url",
        "kind",
        "authority",
        "tier",
        "interval_seconds",
    }
    for source in sources:
        if not isinstance(source, dict) or required - source.keys():
            raise ValueError(f"invalid source fields: {source!r}")
        source_id = str(source["id"])
        url = str(source["url"])
        if source_id in seen_ids or url in seen_urls:
            raise ValueError(f"duplicate source: {source_id} {url}")
        if not url.startswith("https://"):
            raise ValueError(f"source must use HTTPS: {url}")
        if source["kind"] not in {"web", "rss"}:
            raise ValueError(f"unsupported kind for {source_id}: {source['kind']}")
        if source["authority"] != "primary" or source["tier"] != 1:
            raise ValueError(f"source is not primary tier 1: {source_id}")
        interval = source["interval_seconds"]
        if not isinstance(interval, int) or not 3600 <= interval <= 604800:
            raise ValueError(f"invalid interval_seconds for {source_id}")
        seen_ids.add(source_id)
        seen_urls.add(url)
    return sources


def command_for(source: dict[str, object], synx: str, database: Path) -> list[str]:
    return [
        synx,
        "corpus",
        "watch-url",
        str(source["url"]),
        "--file",
        str(database),
        "--kind",
        str(source["kind"]),
        "--title",
        str(source["title"]),
        "--every-secs",
        str(source["interval_seconds"]),
    ]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--registry", type=Path, default=DEFAULT_REGISTRY)
    parser.add_argument("--file", type=Path, default=DEFAULT_DB)
    parser.add_argument(
        "--synx", default="/Users/master/projects/synapse/target/release/synx"
    )
    parser.add_argument("--apply", action="store_true")
    args = parser.parse_args()

    try:
        sources = load_registry(args.registry)
        commands = [command_for(source, args.synx, args.file) for source in sources]
        if args.apply:
            for command in commands:
                subprocess.run(command, check=True, capture_output=True, text=True)
        print(
            json.dumps(
                {
                    "status": "applied" if args.apply else "preview",
                    "source_count": len(sources),
                    "database": str(args.file),
                    "commands": commands if not args.apply else [],
                },
                indent=2,
            )
        )
        return 0
    except (
        OSError,
        ValueError,
        json.JSONDecodeError,
        subprocess.CalledProcessError,
    ) as error:
        print(json.dumps({"status": "blocked", "error": str(error)}), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
