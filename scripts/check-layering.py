#!/usr/bin/env python3
"""Layering guard (ADR 0001).

Product (workspace member) crates must NOT depend on excluded *experimental*
crates under `crates/`. Substrate under `vendor/` is allowed — that is the L0
layer everything legitimately builds on.

Run from anywhere: `python3 scripts/check-layering.py`. Exit 1 on violation.
"""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys

import tomllib

ROOT = pathlib.Path(__file__).resolve().parent.parent


def main() -> int:
    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text())
    excluded = cargo["workspace"].get("exclude", [])
    # Forbidden = excluded crates living under crates/ (experimental/opt-in).
    # vendor/* substrate is intentionally allowed and not flagged.
    forbidden = {pathlib.PurePath(e).name for e in excluded if e.startswith("crates/")}

    meta = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"],
            cwd=ROOT,
        )
    )
    members = sorted(p["name"] for p in meta["packages"])

    # Resolve each dependency by PATH, not name: identical package names exist
    # twice (product island in crates/ vs vendored substrate). A dep that lands
    # under vendor/ is L0 substrate and allowed; only deps resolving into an
    # excluded crates/ dir are violations. Non-path deps keep name-matching.
    violations = []
    for p in meta["packages"]:
        for d in p["dependencies"]:
            if d["name"] not in forbidden:
                continue
            dep_path = d.get("path")
            if dep_path is not None:
                try:
                    rel = pathlib.Path(dep_path).resolve().relative_to(ROOT)
                except ValueError:
                    continue  # dep lives outside the repo tree — not ours
                if not str(rel).startswith("crates/"):
                    continue  # vendored substrate — allowed
                if str(rel) not in excluded:
                    continue  # resolves to a non-excluded crate
            violations.append((p["name"], d["name"]))

    if violations:
        print(
            "LAYERING VIOLATION (ADR 0001): product crate -> excluded experimental crate"
        )
        for member, dep in sorted(violations):
            print(f"  {member} -> {dep}")
        print(
            f"\n{len(violations)} violation(s). Either move the dependency behind "
            "the product boundary, or graduate the crate out of `exclude` and "
            "record it in docs/adr/0001-context-os-product-boundary.md."
        )
        return 1

    print(
        f"layering ok: {len(members)} product crates, "
        f"none depend on {len(forbidden)} excluded crates"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
