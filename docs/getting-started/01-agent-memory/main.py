"""
01-agent-memory — Synapse as persistent AI agent memory.

Requires: synapse-py built via `maturin develop -p synapse-py --release`
Or fall back to CLI mode (see cli_fallback() below).
"""

import subprocess
import sys
import json


def demo_via_cli(db_path: str = "./memory.db") -> None:
    """Pure CLI fallback — works without Python bindings."""
    def synx(*args):
        result = subprocess.run(
            ["synapse", "-f", db_path, *args],
            capture_output=True, text=True
        )
        if result.returncode != 0:
            print(f"[warn] {result.stderr.strip()}", file=sys.stderr)
        return result.stdout.strip()

    # Init store
    synx("init")
    print("Store initialised:", db_path)

    # Seed memories
    memories = [
        ("mem-1", "session started for user alice", {"session": "abc", "role": "system"}),
        ("mem-2", "user said hello",               {"session": "abc", "role": "user"}),
        ("mem-3", "response: hi there!",            {"session": "abc", "role": "assistant"}),
        ("mem-4", "user asked about weather",        {"session": "abc", "role": "user"}),
        ("mem-5", "agent initialized for session xyz", {"session": "xyz", "role": "system"}),
    ]

    for doc_id, text, _meta in memories:
        synx("put", "--uri", doc_id, "--text", text)

    print(f"Stored {len(memories)} memories\n")

    # Search
    print("Top results for 'greetings':")
    raw = synx("find", "greetings", "--limit", "3")
    for line in raw.splitlines():
        if line.strip():
            print(" ", line)

    print("\nHybrid search for 'user hello session':")
    raw = synx("hybrid", "user hello session", "--limit", "3")
    for line in raw.splitlines():
        if line.strip():
            print(" ", line)


def demo_via_python(db_path: str = "./memory.db") -> None:
    """High-level Python API via synapse_rs (maturin build required)."""
    from synapse_rs import Synapse  # type: ignore

    s = Synapse(db_path)

    memories = [
        ("mem-1", "session started for user alice", {"session": "abc", "role": "system"}),
        ("mem-2", "user said hello",               {"session": "abc", "role": "user"}),
        ("mem-3", "response: hi there!",            {"session": "abc", "role": "assistant"}),
        ("mem-4", "user asked about weather",        {"session": "abc", "role": "user"}),
        ("mem-5", "agent initialized for session xyz", {"session": "xyz", "role": "system"}),
    ]

    for doc_id, text, meta in memories:
        s.put(doc_id, text, metadata=meta)

    print(f"Stored {len(memories)} memories\n")

    print("Top results for 'greetings':")
    for doc_id, text, score in s.search("greetings", k=3):
        print(f"  [{score:.2f}] {text}")

    print("\nRecent session abc memories:")
    # Filter is done client-side after lex search — server-side filters in v1.1
    hits = s.search("session abc", k=10)
    for _, text, _ in hits:
        print(f"  {text}")

    s.close()


if __name__ == "__main__":
    import os
    db = os.environ.get("SYNAPSE_DB", "./memory.db")

    try:
        demo_via_python(db)
    except ImportError:
        print("[info] synapse_rs not installed, falling back to CLI mode")
        demo_via_cli(db)
