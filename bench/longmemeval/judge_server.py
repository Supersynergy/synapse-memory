#!/usr/bin/env python3
"""Persistent MLX judge server. Reads JSON-line requests from stdin,
writes JSON-line responses to stdout. Loads the model once.

Request:  {"q": "...", "a": "...", "passages": ["...", ...]}
Response: {"correct": true|false}  (or {"error": "..."} on failure)
"""
import json
import os
import sys

os.environ.setdefault("HF_HUB_OFFLINE", "1")

from mlx_lm import load, generate

MODEL = os.environ.get("LME_JUDGE_MODEL", "mlx-community/Llama-3.2-3B-Instruct-4bit")
MAX_PASS_CHARS = int(os.environ.get("LME_JUDGE_PASS_CHARS", "3000"))
MAX_TOKENS = int(os.environ.get("LME_JUDGE_MAX_TOKENS", "16"))
# Keep query/answer-relevant snippets first by extracting around keyword hits.
USE_SMART_TRUNCATE = os.environ.get("LME_JUDGE_SMART_TRUNCATE", "1") == "1"


import re as _re


_STOP = {
    "the","and","for","are","but","not","you","all","can","had","her","was",
    "one","our","out","day","get","has","him","his","how","man","new","now",
    "old","see","two","way","who","boy","did","its","let","put","say","she",
    "too","use","what","when","where","which","this","that","with","from","have",
    "your","they","their","would","could","should","about","into","than","then",
    "been","were","will","much","many","some","such","only","very","just","also",
    "make","made","does","doing","didnt","mentioned","question","answer",
}


def _smart_truncate(passage: str, q: str, a: str, max_chars: int) -> str:
    if len(passage) <= max_chars:
        return passage
    # Build keyword set from question + answer (alnum tokens >=3 chars,
    # stopwords pruned to keep selectivity).
    toks = []
    seen = set()
    for src in (a, q):  # answer tokens weighted by appearing first
        for t in _re.split(r"[^a-zA-Z0-9]+", src.lower()):
            if len(t) >= 3 and t not in _STOP and t not in seen:
                toks.append(t)
                seen.add(t)
    if not toks:
        return passage[:max_chars]
    plow = passage.lower()
    # Collect ALL hit positions (per token) then pick the window with max
    # density (sliding window of size max_chars).
    hits: list[int] = []
    for tok in toks:
        start = 0
        while True:
            i = plow.find(tok, start)
            if i < 0:
                break
            hits.append(i)
            start = i + len(tok)
    if not hits:
        return passage[:max_chars]
    hits.sort()
    # For each hit, count how many other hits fall within max_chars window
    # centred on it. Pick the densest centre.
    best_centre = hits[0]
    best_count = 0
    half = max_chars // 2
    for c in hits:
        lo, hi = c - half, c + half
        cnt = sum(1 for h in hits if lo <= h <= hi)
        if cnt > best_count:
            best_count = cnt
            best_centre = c
    start = max(0, best_centre - half)
    end = min(len(passage), start + max_chars)
    snippet = passage[start:end]
    if start > 0:
        snippet = "..." + snippet
    if end < len(passage):
        snippet = snippet + "..."
    return snippet


def build_prompt(q: str, a: str, passages: list[str]) -> str:
    joined = ""
    for i, p in enumerate(passages):
        if USE_SMART_TRUNCATE:
            snippet = _smart_truncate(p, q, a, MAX_PASS_CHARS).strip()
        else:
            snippet = p[:MAX_PASS_CHARS].strip()
        joined += f"[Passage {i+1}] {snippet}\n"
    return (
        "Task: You are checking whether a set of retrieved passages contains "
        "the information that supports a known correct answer to a question. "
        "This is an evidence-presence check, NOT a re-derivation of the answer.\n"
        "Output YES if ANY of the following are true:\n"
        "  (a) A passage explicitly states the answer (exact or paraphrased).\n"
        "  (b) A passage states a fact that simply implies the answer "
        "(e.g. '18th birthday' supports answer '18'; 'Data Science certification' "
        "supports answer 'Data Science'; 'I bought sculpting tools' supports answer "
        "'I got my own set of sculpting tools').\n"
        "  (c) For sum/total/count questions: the component numbers or events "
        "are mentioned in the passages even if the final total is not stated.\n"
        "Output NO only if NONE of the passages contain or imply the answer.\n"
        "Be GENEROUS — when in doubt, output YES. The known correct answer is "
        "given to you; you just need to verify the supporting evidence is present "
        "in the passages.\n"
        "Reply with EXACTLY one word: YES or NO. No explanation.\n\n"
        f"Question: {q.strip()}\n"
        f"Known correct answer: {a.strip()}\n"
        f"Retrieved passages:\n{joined}\n"
        "Is the evidence present (YES or NO)?"
    )


def parse_verdict(s: str) -> bool | None:
    low = s.lower().strip()
    # Take first 32 chars
    head = low[:64]
    if "yes" in head and "no" not in head.split("yes", 1)[0]:
        return True
    if head.startswith("yes"):
        return True
    if head.startswith("no"):
        return False
    if "true" in head:
        return True
    if "false" in head:
        return False
    if " no" in head or "no." in head or "no," in head:
        return False
    if " yes" in head:
        return True
    return None


def main():
    sys.stderr.write(f"[judge_server] loading {MODEL} ...\n")
    sys.stderr.flush()
    model, tokenizer = load(MODEL)
    sys.stderr.write("[judge_server] ready\n")
    sys.stderr.flush()
    sys.stdout.write(json.dumps({"ready": True}) + "\n")
    sys.stdout.flush()

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
            prompt = build_prompt(req["q"], req["a"], req["passages"])
            # Apply chat template if available
            try:
                msgs = [{"role": "user", "content": prompt}]
                prompt2 = tokenizer.apply_chat_template(
                    msgs, add_generation_prompt=True, tokenize=False
                )
            except Exception:
                prompt2 = prompt
            out = generate(
                model,
                tokenizer,
                prompt=prompt2,
                max_tokens=MAX_TOKENS,
                verbose=False,
            )
            verdict = parse_verdict(out)
            if verdict is None:
                sys.stdout.write(json.dumps({"correct": False, "raw": out[:120], "parse_fail": True}) + "\n")
            else:
                sys.stdout.write(json.dumps({"correct": verdict, "raw": out[:120]}) + "\n")
        except Exception as e:
            sys.stdout.write(json.dumps({"error": str(e)}) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
