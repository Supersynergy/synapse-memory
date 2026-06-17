#!/usr/bin/env python3
"""code_graph — turn a code repo into a PERSISTENT, queryable Synapse graph.

What grepgod/codegraph do one-shot (extract who-calls-whom), this ingests INTO
Synapse so it survives, fuses with semantic recall, and is agent-queryable:

  python3 scripts/code_graph.py <repo> [--lang rust] [--max 300]

Then:
  synx graph traverse <fn_id>     # call-chain (multi-hop)
  synx graph pagerank             # hottest / most-central functions
  synx graph path <a_id> <b_id>   # call route a -> b
  synx graph communities          # module clusters
  synx hybrid "where is X handled" # semantic recall, fused with the call graph

Self-contained: ripgrep for extraction, synx for storage. No AST/LSP/scip needed.
Each function becomes a Synapse doc (so it's recall-able) AND a graph node; each
call becomes a 'calls' edge. Caller is the function whose body the call sits in.
"""

import json
import re
import subprocess
import sys
from pathlib import Path

LANG = {
    "rust": {"ext": "rs", "def": re.compile(r"\bfn\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*[(<]")},
    "python": {
        "ext": "py",
        "def": re.compile(r"^\s*def\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*\("),
    },
    "ts": {
        "ext": "ts",
        "def": re.compile(r"\bfunction\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*\("),
    },
}
CALL = re.compile(r"\b([a-zA-Z_][a-zA-Z0-9_]*)\s*\(")
# generic noise to never treat as a function symbol
NOISE = {
    "if",
    "while",
    "for",
    "match",
    "return",
    "let",
    "fn",
    "def",
    "function",
    "print",
    "println",
    "assert",
    "Some",
    "Ok",
    "Err",
    "None",
    "vec",
    "format",
}


def rg_files(repo: str, ext: str) -> list[str]:
    out = subprocess.run(
        ["rg", "--files", "-g", f"*.{ext}", repo], capture_output=True, text=True
    ).stdout
    return [l for l in out.splitlines() if "/target/" not in l and "/tests/" not in l]


def extract(files: list[str], lang: dict):
    """Return (defs: name->{file,line,sig}, edges: set[(caller,callee)])."""
    defs: dict[str, dict] = {}
    occ: list[tuple[str, str, int, str]] = []  # (file, current_fn, lineno, line)
    for f in files:
        cur = None
        try:
            lines = Path(f).read_text(errors="ignore").splitlines()
        except OSError:
            continue
        for i, line in enumerate(lines, 1):
            m = lang["def"].search(line)
            if m:
                name = m.group(1)
                cur = name
                defs.setdefault(name, {"file": f, "line": i, "sig": line.strip()[:160]})
            occ.append((f, cur, i, line))
    names = set(defs)
    edges = set()
    for _f, cur, _i, line in occ:
        if not cur:
            continue
        for c in CALL.findall(line):
            if c in names and c != cur and c not in NOISE:
                edges.add((cur, c))
    return defs, edges


def synx(args: list[str], inp: str | None = None) -> str:
    return subprocess.run(
        ["synx", *args], input=inp, capture_output=True, text=True
    ).stdout


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return
    repo = sys.argv[1]
    lang = sys.argv[sys.argv.index("--lang") + 1] if "--lang" in sys.argv else "rust"
    mx = int(sys.argv[sys.argv.index("--max") + 1]) if "--max" in sys.argv else 300
    files = rg_files(repo, LANG[lang]["ext"])
    defs, edges = extract(files, LANG[lang])
    print(
        f"extracted: {len(defs)} functions, {len(edges)} call-edges from {len(files)} {lang} files"
    )

    # Ingest functions as docs (capped) -> name->doc_id map.
    name_id: dict[str, int] = {}
    items = list(defs.items())[:mx]
    for name, d in items:
        body = f"code:fn {name} @ {d['file']}:{d['line']} :: {d['sig']}"
        out = synx(["put", "--title", f"fn:{name}"], inp=body)
        try:
            name_id[name] = json.loads(out).get("Id")
        except (json.JSONDecodeError, AttributeError):
            pass
    print(f"ingested {len(name_id)} function-nodes as Synapse docs")

    # Build call-edges between ingested nodes.
    n_edges = 0
    for caller, callee in edges:
        a, b = name_id.get(caller), name_id.get(callee)
        if a and b and a != b:
            synx(["graph", "relate", str(a), str(b), "calls"])
            n_edges += 1
    print(f"related {n_edges} 'calls' edges into the Synapse graph")
    print("\nquery it:")
    sample = next(iter(name_id.items()), None)
    if sample:
        print(f"  synx graph traverse {sample[1]}    # call-chain from fn:{sample[0]}")
    print("  synx graph pagerank              # hottest functions")
    print("  synx graph count                 # total edges")


if __name__ == "__main__":
    main()
