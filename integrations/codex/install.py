#!/usr/bin/env python3
"""Install or remove Synapse crash-safe hooks for Codex."""

from __future__ import annotations

import argparse
import json
import os
import shlex
import shutil
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
import tempfile


EVENTS = {
    "SessionStart": ("startup|resume", "session-start"),
    "PreToolUse": (
        r"Bash|Shell|shell|shell_command|exec_command|functions\.exec_command|apply_patch|Write|Edit|MultiEdit",
        "pre-tool",
    ),
    "PostToolUse": (
        r"Bash|Shell|shell|shell_command|exec_command|functions\.exec_command|apply_patch|Write|Edit|MultiEdit",
        "post-tool",
    ),
    "Stop": (".*", "stop"),
}


def _args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("install", "uninstall"), nargs="?", default="install")
    parser.add_argument("--codex-home", type=Path, default=Path.home() / ".codex")
    parser.add_argument("--dry-run", action="store_true")
    return parser.parse_args()


def _load(path: Path) -> dict[str, Any]:
    if not path.exists():
        return {"hooks": {}}
    value = json.loads(path.read_text())
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    hooks = value.setdefault("hooks", {})
    if not isinstance(hooks, dict):
        raise ValueError(f"{path}: hooks must be a JSON object")
    return value


def _is_synapse_checkpoint(entry: Any) -> bool:
    if not isinstance(entry, dict):
        return False
    hooks = entry.get("hooks")
    if not isinstance(hooks, list):
        return False
    return any(
        isinstance(hook, dict) and "synapse-checkpoint.py" in str(hook.get("command", ""))
        for hook in hooks
    )


def _remove_existing(config: dict[str, Any]) -> None:
    hooks = config["hooks"]
    for event in list(hooks):
        entries = hooks[event]
        if not isinstance(entries, list):
            continue
        kept = [entry for entry in entries if not _is_synapse_checkpoint(entry)]
        if kept:
            hooks[event] = kept
        else:
            hooks.pop(event, None)


def _install_entries(config: dict[str, Any], hook_path: Path) -> None:
    python = shlex.quote(str(Path(sys.executable).resolve()))
    hook = shlex.quote(str(hook_path))
    for event, (matcher, mode) in EVENTS.items():
        config["hooks"].setdefault(event, []).append(
            {
                "matcher": matcher,
                "hooks": [
                    {
                        "type": "command",
                        "command": f"{python} {hook} {mode}",
                        "timeout": 3,
                    }
                ],
            }
        )


def _fsync_dir(path: Path) -> None:
    try:
        fd = os.open(path, os.O_RDONLY)
    except OSError:
        return
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def _write_atomic(path: Path, content: bytes, mode: int) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as handle:
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(tmp_name, mode)
        os.replace(tmp_name, path)
        _fsync_dir(path.parent)
    finally:
        try:
            os.unlink(tmp_name)
        except FileNotFoundError:
            pass


def main() -> int:
    args = _args()
    codex_home = args.codex_home.expanduser().resolve(strict=False)
    config_path = codex_home / "hooks.json"
    target = codex_home / "hooks/synapse-checkpoint.py"
    source = Path(__file__).parent / "hooks/checkpoint.py"
    config = _load(config_path)
    _remove_existing(config)
    if args.action == "install":
        _install_entries(config, target)

    rendered = json.dumps(config, indent=2, ensure_ascii=False) + "\n"
    if args.dry_run:
        print(rendered, end="")
        return 0

    codex_home.mkdir(parents=True, exist_ok=True)
    if config_path.exists():
        stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
        shutil.copy2(config_path, config_path.with_name(f"hooks.json.bak-synapse-{stamp}"))
    if args.action == "install":
        target.parent.mkdir(parents=True, exist_ok=True)
        _write_atomic(target, source.read_bytes(), 0o755)
        _write_atomic(config_path, rendered.encode(), 0o600)
        print(f"installed: {target}")
    else:
        _write_atomic(config_path, rendered.encode(), 0o600)
        target.unlink(missing_ok=True)
        print("removed Synapse checkpoint hook entries")
    print(f"updated: {config_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
