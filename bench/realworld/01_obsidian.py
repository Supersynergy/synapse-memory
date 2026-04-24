"""Obsidian-vault bench — a01.

Usage:
    python 01_obsidian.py ~/Vaults/MyNotes

Produces the "consumer metric" table:
    • "find any note across 50k markdown files in 1.2 ms (95 %)"

Wrapped on top of harness.py — vault = dir of markdown files, queries = 20
random note-titles pulled from the vault.
"""

from __future__ import annotations
import sys
from pathlib import Path
from harness import main as harness_main


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: python 01_obsidian.py <vault-path> [extra harness args]", file=sys.stderr)
        sys.exit(1)
    vault = Path(sys.argv[1]).expanduser().resolve()
    extra = sys.argv[2:]
    raise SystemExit(harness_main(["--src", str(vault), "--dim", "128", *extra]))
