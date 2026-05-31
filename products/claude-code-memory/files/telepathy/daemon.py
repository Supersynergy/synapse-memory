#!/usr/bin/env python3
"""
Synapse Telepathy — cross-session memory for Claude Code.

Tails all Claude Code jsonl transcripts in ~/.claude/projects/**/*.jsonl,
extracts compact events (prompt / reply / tool calls), and pushes them into
Synapse via `synx put`. A companion hook re-injects recent activity from OTHER
sessions on SessionStart + UserPromptSubmit, giving every live Claude
session shared awareness of what the other parallel sessions are doing.

Zero-deps (stdlib only). Prefers `synx-fast` on PATH, falls back to `synx`.
"""
import json
import os
import time
import glob
import shutil
import subprocess
from pathlib import Path

STATE = Path.home() / ".claude/telepathy/offsets.json"
LOGF  = Path.home() / ".claude/telepathy/daemon.log"
PROJ  = Path.home() / ".claude/projects"
POLL  = float(os.environ.get("TELEPATHY_POLL", "4.0"))
BATCH_MAX = int(os.environ.get("TELEPATHY_BATCH", "200"))
MAX_LINE_SCAN = int(os.environ.get("TELEPATHY_MAX_LINES", "500"))
IDLE_CUTOFF   = int(os.environ.get("TELEPATHY_IDLE_CUTOFF", "1800"))


def resolve_synx_bin() -> str:
    explicit = os.environ.get("SYNX_WRITE_BIN") or os.environ.get("SYNX_BIN")
    if explicit:
        return explicit
    return (
        shutil.which("synx-fast")
        or ("/Users/master/.local/bin/synx-fast" if Path("/Users/master/.local/bin/synx-fast").exists() else None)
        or shutil.which("synx")
        or "synx"
    )


SYNX_BIN      = resolve_synx_bin()


def log(msg: str) -> None:
    LOGF.parent.mkdir(parents=True, exist_ok=True)
    with open(LOGF, "a") as f:
        f.write(f"{time.strftime('%H:%M:%S')} {msg}\n")


def load_state() -> dict:
    if STATE.exists():
        try:
            return json.loads(STATE.read_text())
        except Exception:
            pass
    return {}


def save_state(s: dict) -> None:
    STATE.parent.mkdir(parents=True, exist_ok=True)
    STATE.write_text(json.dumps(s))


def extract(ev: dict) -> str | None:
    """Return a compact single-line string describing the event, or None."""
    t = ev.get("type")
    sid = (ev.get("sessionId") or "")[:8]
    if not sid:
        return None
    cwd = (ev.get("cwd", "") or "").split("/")[-1] or "-"
    msg = ev.get("message")

    if t == "user" and isinstance(msg, dict):
        c = msg.get("content")
        if isinstance(c, str):
            txt = c.strip()
        elif isinstance(c, list):
            parts = [x.get("text", "") for x in c
                     if isinstance(x, dict) and x.get("type") == "text"]
            txt = " ".join(parts).strip()
        else:
            txt = ""
        if txt and not txt.startswith("<") and len(txt) > 3:
            return f"[telepathy][{sid}][{cwd}][prompt] {txt[:240]}"

    elif t == "assistant" and isinstance(msg, dict):
        c = msg.get("content", [])
        if isinstance(c, list):
            texts, tools = [], []
            for x in c:
                if not isinstance(x, dict):
                    continue
                if x.get("type") == "text":
                    texts.append(x.get("text", "")[:200])
                elif x.get("type") == "tool_use":
                    tools.append(x.get("name", ""))
            if texts:
                return f"[telepathy][{sid}][{cwd}][reply] {' '.join(texts)[:240]}"
            if tools:
                return f"[telepathy][{sid}][{cwd}][tools] {','.join(tools[:6])}"
    return None


def push(text: str) -> None:
    """Single put — kept for compatibility but prefer push_batch."""
    try:
        subprocess.run([SYNX_BIN, "put", text], timeout=8, capture_output=True)
    except Exception as e:
        log(f"push_err {e}")


def project_scope(text: str) -> str | None:
    explicit = os.environ.get("TELEPATHY_SCOPE")
    if explicit:
        return explicit
    if os.environ.get("TELEPATHY_NO_SCOPE") == "1":
        return None
    parts = text.split("]", 4)
    if len(parts) >= 3:
        cwd = parts[2].lstrip("[")
        if cwd and cwd != "-":
            return cwd[:80]
    return None


def push_batch(texts: list[str]) -> None:
    """Batch put as JSONL to avoid N socket roundtrips."""
    if not texts:
        return
    lines = []
    for text in texts:
        title = text[:96]
        meta = {"source": "telepathy"}
        scope = project_scope(text)
        if scope:
            meta["scope"] = scope
        lines.append(json.dumps({"title": title, "text": text, "meta": meta, "embed": True}, ensure_ascii=False))
    payload = "\n".join(lines).encode()[:1_000_000]  # 1MB cap
    try:
        subprocess.run([SYNX_BIN, "put-batch"], input=payload, timeout=15, capture_output=True)
    except Exception as e:
        log(f"push_batch_err {e}")


def tick(state: dict) -> None:
    now = time.time()
    files = glob.glob(str(PROJ / "**/*.jsonl"), recursive=True)
    batch: list[str] = []
    seen: set[str] = set()
    emitted = 0
    for fp in files:
        try:
            st = os.stat(fp)
            if now - st.st_mtime > IDLE_CUTOFF:
                continue
            first_seen = fp not in state
            off = state.get(fp, st.st_size)
            if off > st.st_size:
                off = 0  # rotation / truncation
            state[fp] = st.st_size
            if first_seen or off == st.st_size:
                continue
            with open(fp, "rb") as f:
                f.seek(off)
                data = f.read()
            lines = data.split(b"\n")
            for ln in lines[-MAX_LINE_SCAN:]:
                if not ln.strip():
                    continue
                try:
                    ev = json.loads(ln)
                except Exception:
                    continue
                msg = extract(ev)
                if msg and msg not in seen:
                    seen.add(msg)
                    batch.append(msg)
                    emitted += 1
                    if len(batch) >= BATCH_MAX:
                        push_batch(batch); batch = []
        except Exception as e:
            log(f"tick_err {fp} {e}")
    if batch:
        push_batch(batch)
    if emitted:
        log(f"emitted {emitted} (batched)")


def main() -> None:
    log("daemon_start")
    state = load_state()
    while True:
        try:
            tick(state)
            save_state(state)
        except Exception as e:
            log(f"main_err {e}")
        time.sleep(POLL)


if __name__ == "__main__":
    main()
