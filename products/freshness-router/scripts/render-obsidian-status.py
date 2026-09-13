#!/usr/bin/env python3
"""Render the Synapse source-watch state as a human-reviewable Obsidian note."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import sqlite3
import time
from pathlib import Path


PRODUCT_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_REGISTRY = PRODUCT_ROOT / "files" / "primary-sources.json"
DEFAULT_DB = Path.home() / ".synapse" / "brain.db"
DEFAULT_PRIMEQUELLEN_DB = Path.home() / "research" / "primequellen" / "primequellen.db"
DEFAULT_NOTE = (
    Path.home()
    / "Documents"
    / "Obsidian Vault"
    / "03_Resources"
    / "Knowledge Freshness Control Plane.md"
)


def iso_time(value: int | None) -> str:
    if value is None:
        return "never"
    return dt.datetime.fromtimestamp(value, tz=dt.UTC).strftime("%Y-%m-%d %H:%M UTC")


def source_rows(database: Path, urls: list[str]) -> dict[str, sqlite3.Row]:
    connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True)
    connection.row_factory = sqlite3.Row
    rows = connection.execute(
        """
        SELECT s.uri, s.last_sync_ts, s.sync_interval_secs,
               COUNT(d.id) AS document_count,
               MAX(d.created_ts) AS latest_document_ts
        FROM synapse_corpus_sources AS s
        LEFT JOIN synapse_corpus_documents AS d ON d.source_id = s.id
        GROUP BY s.id
        """
    ).fetchall()
    connection.close()
    allowed = set(urls)
    return {str(row["uri"]): row for row in rows if str(row["uri"]) in allowed}


def primequellen_summary(database: Path) -> dict[str, object]:
    connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True)
    connection.row_factory = sqlite3.Row
    totals = connection.execute(
        """
        SELECT (SELECT COUNT(*) FROM observations) AS observations,
               (SELECT COUNT(*) FROM series_meta) AS series,
               (SELECT MAX(last_run) FROM series_meta) AS latest_series_run
        """
    ).fetchone()
    source_states = {
        str(row["last_status"]): int(row["count"])
        for row in connection.execute(
            "SELECT last_status, COUNT(*) AS count FROM sources GROUP BY last_status"
        )
    }
    latest_source_check = connection.execute(
        "SELECT MAX(last_run) FROM sources"
    ).fetchone()[0]
    connection.close()
    latest_series_run = totals["latest_series_run"]
    series_age_seconds = None
    if latest_series_run:
        series_age_seconds = int(
            (
                dt.datetime.now(tz=dt.UTC)
                - dt.datetime.fromisoformat(str(latest_series_run))
            ).total_seconds()
        )
    return {
        "observations": int(totals["observations"]),
        "series": int(totals["series"]),
        "latest_series_run": latest_series_run,
        "series_status": (
            "current"
            if series_age_seconds is not None and series_age_seconds <= 604800
            else "stale"
        ),
        "latest_source_check": latest_source_check,
        "source_ok": source_states.get("OK", 0),
        "source_failed": sum(
            count
            for status, count in source_states.items()
            if status.startswith("FAIL")
        ),
        "source_standby": sum(
            count
            for status, count in source_states.items()
            if status.startswith("STANDBY")
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--registry", type=Path, default=DEFAULT_REGISTRY)
    parser.add_argument("--file", type=Path, default=DEFAULT_DB)
    parser.add_argument(
        "--primequellen-file", type=Path, default=DEFAULT_PRIMEQUELLEN_DB
    )
    parser.add_argument("--note", type=Path, default=DEFAULT_NOTE)
    args = parser.parse_args()

    payload = json.loads(args.registry.read_text(encoding="utf-8"))
    sources = payload["sources"]
    now = int(time.time())
    rows = source_rows(args.file, [str(source["url"]) for source in sources])
    primequellen = primequellen_summary(args.primequellen_file)
    status_counts = {"current": 0, "stale": 0, "pending": 0, "blocked": 0}
    table = []
    for source in sources:
        url = str(source["url"])
        row = rows.get(url)
        if row is None or row["last_sync_ts"] is None:
            status = "pending"
            last_sync = None
            document_count = 0
        else:
            last_sync = int(row["last_sync_ts"])
            interval = int(row["sync_interval_secs"] or source["interval_seconds"])
            document_count = int(row["document_count"])
            if document_count == 0:
                status = "blocked"
            else:
                status = "current" if last_sync + interval > now else "stale"
        status_counts[status] += 1
        table.append(
            f"| {source['title']} | {status} | {document_count} | "
            f"{iso_time(last_sync)} | [source]({url}) |"
        )

    rendered = "\n".join(
        [
            "---",
            "type: source-note",
            "status: active",
            "tags:",
            "  - synapse",
            "  - knowledge-freshness",
            "---",
            "",
            "# Knowledge Freshness Control Plane",
            "",
            f"Generated: {iso_time(now)}",
            "",
            f"- Current: {status_counts['current']}",
            f"- Stale: {status_counts['stale']}",
            f"- Pending: {status_counts['pending']}",
            f"- Blocked: {status_counts['blocked']}",
            "- Canonical state: `/Users/master/.synapse/brain.db`",
            "- Registry: `/Users/master/BASE/projects/synapse/products/freshness-router/files/primary-sources.json`",
            "- Discovery-only signal: ghmax; never sufficient for verification",
            "",
            "| Source | State | Documents | Last sync | Primary URL |",
            "|---|---:|---:|---|---|",
            *table,
            "",
            "## Primequellen",
            "",
            "Role: independent market and macro evidence store; not a general-purpose verifier.",
            "",
            f"- Observations: {primequellen['observations']}",
            f"- Series: {primequellen['series']}",
            f"- Latest series run: {primequellen['latest_series_run']}",
            f"- Series status (7-day TTL): {primequellen['series_status']}",
            f"- Latest source check: {primequellen['latest_source_check']}",
            f"- Source access: {primequellen['source_ok']} OK, {primequellen['source_failed']} failed, {primequellen['source_standby']} standby",
            "- Canonical store: `/Users/master/research/primequellen/primequellen.db`",
            "",
            "## Gate",
            "",
            "A volatile claim is usable only when its evidence is primary, hashed, inside TTL, and reviewed. Fetch failures remain pending or blocked.",
            "",
        ]
    )
    args.note.parent.mkdir(parents=True, exist_ok=True)
    temporary = args.note.with_suffix(".md.tmp")
    temporary.write_text(rendered, encoding="utf-8")
    temporary.replace(args.note)
    print(
        json.dumps(
            {"note": str(args.note), **status_counts, "primequellen": primequellen},
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
