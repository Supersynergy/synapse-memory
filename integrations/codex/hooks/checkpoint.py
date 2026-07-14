#!/usr/bin/env python3
"""Crash-safe, content-minimal checkpoints for Codex sessions.

The hook stores execution state, never transcript or tool-output bodies.  Each
record is appended and fsynced before a compact per-project snapshot is
atomically replaced.  A later SessionStart can therefore explain exactly what
must be verified after an interrupted turn without replaying a mutation.
"""

from __future__ import annotations

import hashlib
import html
import json
import os
import shlex
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Optional

try:
    import fcntl
except ImportError:  # pragma: no cover - Codex desktop currently targets macOS/Linux.
    fcntl = None  # type: ignore[assignment]


SCHEMA_VERSION = 1
MAX_DIRTY_PATHS = 40
MAX_AGE_SECONDS = 7 * 24 * 60 * 60
MUTATING_EVENTS = {"pre-tool", "post-tool", "stop"}


def _text(value: Any) -> str:
    return value if isinstance(value, str) else ""


def _context_text(value: Any, limit: int = 500) -> str:
    raw = str(value)
    clean = "".join(ch if ch.isprintable() and ch not in "\r\n\t" else " " for ch in raw)
    return html.escape(clean[:limit], quote=True)


def _read_event() -> dict[str, Any]:
    try:
        value = json.load(sys.stdin)
    except (json.JSONDecodeError, OSError):
        return {}
    return value if isinstance(value, dict) else {}


def _cwd(event: dict[str, Any]) -> Path:
    raw = _text(event.get("cwd")) or os.environ.get("PWD", "") or os.getcwd()
    return Path(raw).expanduser().resolve(strict=False)


def _state_dir() -> Path:
    override = os.environ.get("SYNAPSE_CHECKPOINT_DIR")
    return Path(override).expanduser() if override else Path.home() / ".synapse/checkpoints"


def _project_key(cwd: Path) -> str:
    return hashlib.sha256(os.fsencode(cwd)).hexdigest()[:20]


def _run_git(cwd: Path, *args: str) -> str:
    try:
        result = subprocess.run(
            ["git", "-C", str(cwd), *args],
            capture_output=True,
            text=True,
            timeout=1.5,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        return ""
    return result.stdout.strip() if result.returncode == 0 else ""


def _git_state(cwd: Path) -> dict[str, Any]:
    root = _run_git(cwd, "rev-parse", "--show-toplevel")
    if not root:
        return {}
    head = _run_git(cwd, "rev-parse", "--short=12", "HEAD")
    status = _run_git(cwd, "status", "--porcelain=v1", "--untracked-files=normal")
    paths: list[str] = []
    for line in status.splitlines():
        if len(line) < 4:
            continue
        path = line[3:]
        if " -> " in path:
            path = path.split(" -> ", 1)[1]
        paths.append(path)
        if len(paths) == MAX_DIRTY_PATHS:
            break
    return {
        "root": root,
        "head": head,
        "dirty_count": len(status.splitlines()),
        "dirty_paths": paths,
    }


def _tool_summary(event: dict[str, Any]) -> dict[str, Any]:
    tool_name = _text(event.get("tool_name")) or _text(event.get("tool"))
    raw_input = event.get("tool_input")
    tool_input = raw_input if isinstance(raw_input, dict) else {}
    command = _text(tool_input.get("command")) or _text(tool_input.get("cmd"))
    command_verb = ""
    if command:
        try:
            words = shlex.split(command, posix=True)
        except ValueError:
            words = command.split()
        if words:
            command_verb = Path(words[0]).name[:64]
    paths: list[str] = []
    for key in ("file_path", "path", "workdir"):
        value = _text(tool_input.get(key))
        if value:
            paths.append(value[:500])
    summary: dict[str, Any] = {"name": tool_name[:120]}
    if command_verb:
        summary["command_verb"] = command_verb
        summary["command_sha256"] = hashlib.sha256(command.encode()).hexdigest()[:16]
    if paths:
        summary["target_paths"] = paths
    return summary


def _tool_ok(event: dict[str, Any]) -> Optional[bool]:
    response = event.get("tool_response") or event.get("tool_output")
    if not isinstance(response, dict):
        return None
    for key in ("exit_code", "exitCode", "status_code"):
        value = response.get(key)
        if isinstance(value, int):
            return value == 0
    if isinstance(response.get("is_error"), bool):
        return not response["is_error"]
    return None


def _record(kind: str, event: dict[str, Any], cwd: Path) -> dict[str, Any]:
    status = {
        "pre-tool": "in_progress",
        "post-tool": "tool_completed",
        "stop": "completed",
    }[kind]
    session_id = (
        _text(event.get("session_id"))
        or _text(event.get("thread_id"))
        or _text(event.get("conversation_id"))
        or "unknown"
    )
    record: dict[str, Any] = {
        "schema": SCHEMA_VERSION,
        "timestamp": datetime.now(timezone.utc).isoformat(timespec="milliseconds"),
        "timestamp_unix": time.time(),
        "event": kind,
        "status": status,
        "session_id": session_id,
        "cwd": str(cwd),
        "tool": _tool_summary(event),
        "git": _git_state(cwd),
    }
    ok = _tool_ok(event)
    if ok is not None:
        record["tool_ok"] = ok
    return record


def _fsync_dir(path: Path) -> None:
    try:
        fd = os.open(path, os.O_RDONLY)
    except OSError:
        return
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def _persist(record: dict[str, Any], state_dir: Path, key: str) -> None:
    state_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
    state_dir.chmod(0o700)
    line = (json.dumps(record, ensure_ascii=False, separators=(",", ":")) + "\n").encode()
    journal = state_dir / f"{key}.jsonl"
    fd = os.open(journal, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
    try:
        # `mode` only applies on creation; repair a pre-existing permissive file.
        os.fchmod(fd, 0o600)
        if fcntl is not None:
            fcntl.flock(fd, fcntl.LOCK_EX)
        remaining = memoryview(line)
        while remaining:
            written = os.write(fd, remaining)
            if written <= 0:
                raise OSError("checkpoint journal write made no progress")
            remaining = remaining[written:]
        os.fsync(fd)
    finally:
        if fcntl is not None:
            fcntl.flock(fd, fcntl.LOCK_UN)
        os.close(fd)

    latest = state_dir / f"{key}.latest.json"
    tmp_fd, tmp_name = tempfile.mkstemp(prefix=f".{key}.", dir=state_dir)
    try:
        with os.fdopen(tmp_fd, "wb") as handle:
            handle.write(line)
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(tmp_name, 0o600)
        os.replace(tmp_name, latest)
        _fsync_dir(state_dir)
    finally:
        try:
            os.unlink(tmp_name)
        except FileNotFoundError:
            pass


def _load_latest(state_dir: Path, key: str) -> Optional[dict[str, Any]]:
    try:
        value = json.loads((state_dir / f"{key}.latest.json").read_text())
    except (OSError, json.JSONDecodeError):
        return None
    if not isinstance(value, dict) or value.get("schema") != SCHEMA_VERSION:
        return None
    try:
        age = time.time() - float(value.get("timestamp_unix", 0))
    except (TypeError, ValueError):
        return None
    if age < 0 or age > MAX_AGE_SECONDS or value.get("status") == "completed":
        return None
    return value


def _resume_context(record: dict[str, Any]) -> str:
    tool = record.get("tool") if isinstance(record.get("tool"), dict) else {}
    git = record.get("git") if isinstance(record.get("git"), dict) else {}
    tool_name = _text(tool.get("name")) or "unknown tool"
    command_verb = _text(tool.get("command_verb"))
    if command_verb:
        tool_name = f"{tool_name} ({command_verb})"
    return (
        "<synapse_recovery>\n"
        "Unfinished crash-safe checkpoint detected. Do not replay mutations blindly.\n"
        "scope: current working directory\n"
        f"last_event: {_context_text(record.get('event', ''))}\n"
        f"last_tool: {_context_text(tool_name)}\n"
        f"git_head_at_checkpoint: {_context_text(git.get('head', ''))}\n"
        f"dirty_path_count_at_checkpoint: {_context_text(git.get('dirty_count', 0))}\n"
        "resume_rule: inspect current git/files/process state, then continue from the smallest verified delta.\n"
        "</synapse_recovery>"
    )


def _emit_session_context(context: str) -> None:
    json.dump(
        {
            "hookSpecificOutput": {
                "hookEventName": "SessionStart",
                "additionalContext": context,
            }
        },
        sys.stdout,
        ensure_ascii=False,
        separators=(",", ":"),
    )


def main(argv: list[str]) -> int:
    kind = argv[1] if len(argv) > 1 else ""
    if kind not in MUTATING_EVENTS | {"session-start"}:
        return 0
    event = _read_event()
    cwd = _cwd(event)
    state_dir = _state_dir()
    key = _project_key(cwd)
    if kind == "session-start":
        latest = _load_latest(state_dir, key)
        if latest is not None:
            _emit_session_context(_resume_context(latest))
        return 0
    try:
        _persist(_record(kind, event, cwd), state_dir, key)
    except OSError:
        # A checkpoint must never block Codex itself.
        return 0
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
