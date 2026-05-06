#!/usr/bin/env python3.14
"""Ingest last 50 chat session summaries into Synapse via synx."""
import json, sys, subprocess, glob, os, re, time
from collections import Counter
from pathlib import Path

PROJ = Path.home() / ".claude/projects/-Users-master"
files = sorted(PROJ.glob("**/*.jsonl"), key=lambda p: p.stat().st_mtime, reverse=True)[:50]

ingested = 0
skipped = 0
SYNX = "/Users/master/.local/bin/synx"

for f in files:
    sid = f.stem[:16]
    if "subagents" in str(f):
        continue

    # Build a compact summary
    lines_seen = 0
    user_msgs, asst_msgs = [], []
    try:
        with open(f, 'rb') as h:
            for line in h.readlines()[:300]:
                try:
                    d = json.loads(line)
                    msg = d.get('message', {})
                    role = msg.get('role', '')
                    content = msg.get('content', '')
                    if isinstance(content, list):
                        content = ' '.join(str(c.get('text', '')) for c in content if isinstance(c, dict))
                    if not isinstance(content, str) or len(content) < 30:
                        continue
                    if role == 'user' and len(user_msgs) < 5:
                        user_msgs.append(content[:300])
                    elif role == 'assistant' and len(asst_msgs) < 3:
                        asst_msgs.append(content[:200])
                except: pass
    except: continue

    if not user_msgs:
        skipped += 1
        continue

    title = user_msgs[0][:80].replace('\n', ' ')
    body_parts = [f"[chat-cluster][{sid}] {title}"]
    body_parts.extend(f"Q: {m}" for m in user_msgs[:3])
    body_parts.extend(f"A: {m}" for m in asst_msgs[:2])
    body = '\n'.join(body_parts)[:1500]

    # Push via synx put
    r = subprocess.run([SYNX, "put", body], capture_output=True, timeout=10, text=True)
    if r.returncode == 0:
        ingested += 1
    else:
        skipped += 1
        if skipped < 3:
            print(f"FAIL {sid}: {r.stderr[:80]}")

print(f"=== ingested={ingested} skipped={skipped} of {len(files)} files ===")
