#!/usr/bin/env python3.14
"""Cluster last 50 claude chats by topic-keywords + report."""
import json, sys, glob, os, re, time
from collections import Counter
from pathlib import Path

PROJ = Path.home() / ".claude/projects/-Users-master"
files = sorted(PROJ.glob("**/*.jsonl"), key=lambda p: p.stat().st_mtime, reverse=True)[:50]

# Stop words (DE+EN tech-context)
STOP = set("the a an of and or for with to in on at by from up as is are was were be been being have has had do does did".split())
STOP |= set("ich du er sie es wir ihr was wer wann wo wie warum weil aber oder und mit auf für der die das den dem ist sind war".split())
STOP |= set("ja nein ok hi bitte mal noch nur auch schon eben doch hier da nun also dann".split())

clusters = Counter()
session_topics = {}

for f in files:
    sid = f.stem[:8]
    text = []
    try:
        with open(f, 'rb') as h:
            for line in h.readlines()[:200]:
                try:
                    d = json.loads(line)
                    msg = d.get('message', {})
                    content = msg.get('content', '')
                    if isinstance(content, list):
                        content = ' '.join(str(c.get('text', '')) for c in content if isinstance(c, dict))
                    if isinstance(content, str) and len(content) > 20:
                        text.append(content[:500])
                except: pass
    except: continue

    body = ' '.join(text).lower()
    # Extract tech keywords (3+ char words, not stop, occur 2+ times)
    words = re.findall(r'\b[a-z][a-z0-9_-]{2,}\b', body)
    cnt = Counter(w for w in words if w not in STOP)
    top5 = [w for w, n in cnt.most_common(15) if n >= 2][:5]
    if top5:
        session_topics[sid] = top5
        for kw in top5:
            clusters[kw] += 1

# Report
print(f"=== {len(session_topics)} sessions analysed ===")
print(f"\n## Top 30 cluster topics (last 50 sessions)")
for kw, n in clusters.most_common(30):
    print(f"  {n:3d}  {kw}")

print(f"\n## Per-session topic fingerprint (sample 20)")
for sid, kws in list(session_topics.items())[:20]:
    print(f"  {sid}  {' '.join(kws)}")
