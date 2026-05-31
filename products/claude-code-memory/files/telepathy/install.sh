#!/usr/bin/env bash
# Install Synapse Telepathy for Claude Code.
# - Copies daemon + hook into ~/.claude/
# - Renders launchd plist (macOS) and loads it
# - Registers hook into ~/.claude/settings.json (SessionStart + UserPromptSubmit)
set -euo pipefail

SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
CLAUDE_DIR="$HOME/.claude"
SCRIPTS="$CLAUDE_DIR/scripts"
HOOKS="$CLAUDE_DIR/hooks"
STATE_DIR="$CLAUDE_DIR/telepathy"
PY="${PYTHON:-$(command -v python3)}"

mkdir -p "$SCRIPTS" "$HOOKS" "$STATE_DIR"

install -m 0755 "$SRC_DIR/daemon.py"  "$SCRIPTS/telepathy_daemon.py"
install -m 0755 "$SRC_DIR/inject.sh"  "$HOOKS/telepathy_inject.sh"

# Register hooks in settings.json (idempotent)
"$PY" - <<PY
import json, pathlib
p = pathlib.Path.home() / ".claude/settings.json"
d = json.loads(p.read_text()) if p.exists() else {}
h = d.setdefault("hooks", {})
entry = {"hooks":[{"type":"command","command":"$HOOKS/telepathy_inject.sh"}]}
for key in ("SessionStart","UserPromptSubmit"):
    arr = h.setdefault(key, [])
    if not any("telepathy_inject.sh" in json.dumps(x) for x in arr):
        arr.append(entry)
p.write_text(json.dumps(d, indent=2))
print("[ok] hooks registered:", list(h.keys()))
PY

# macOS launchd
if [[ "$(uname)" == "Darwin" ]]; then
  PLIST="$HOME/Library/LaunchAgents/de.supersynergy.telepathy.plist"
  sed -e "s|__PYTHON__|$PY|g" \
      -e "s|__DAEMON__|$SCRIPTS/telepathy_daemon.py|g" \
      -e "s|__HOME__|$HOME|g" \
      "$SRC_DIR/de.supersynergy.telepathy.plist" > "$PLIST"
  launchctl unload "$PLIST" 2>/dev/null || true
  launchctl load   "$PLIST"
  echo "[ok] launchd agent loaded: $PLIST"
else
  echo "[info] non-macOS — run daemon manually: $PY $SCRIPTS/telepathy_daemon.py &"
fi

echo ""
echo "✅ Synapse Telepathy installed."
echo "   daemon : $SCRIPTS/telepathy_daemon.py"
echo "   hook   : $HOOKS/telepathy_inject.sh"
echo "   state  : $STATE_DIR/"
echo ""
echo "Verify: tail -f $STATE_DIR/daemon.log"
