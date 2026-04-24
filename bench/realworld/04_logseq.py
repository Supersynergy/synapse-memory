"""Logseq journal bench — a04.

Logseq stores every page as a markdown file under `pages/` + `journals/`.
This harness just walks the dir and reuses the generic harness.

Usage:
    python 04_logseq.py ~/Documents/Logseq
"""

from __future__ import annotations
import sys
from pathlib import Path
from harness import main as harness_main


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: python 04_logseq.py <logseq-dir>", file=sys.stderr)
        sys.exit(1)
    root = Path(sys.argv[1]).expanduser().resolve()
    extra = sys.argv[2:]
    raise SystemExit(harness_main(["--src", str(root), "--dim", "128", *extra]))
