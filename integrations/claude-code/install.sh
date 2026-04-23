#!/bin/bash
# synapse-claude-code — hook-pack for auto-memory in Claude Code
# Installs: SessionStart (top-8 recall) + UserPromptSubmit (semantic inject) + Stop (decision extract)
set -e

CLAUDE_DIR="${CLAUDE_DIR:-$HOME/.claude}"
HOOK_DIR="$CLAUDE_DIR/hooks/synapse"
SETTINGS="$CLAUDE_DIR/settings.json"

mkdir -p "$HOOK_DIR"

# Check daemon
if [ ! -S /tmp/synapse.sock ]; then
  echo "⚠ synapsed daemon not running. Start: synapsed -f ~/.synapse/brain.db &"
  echo "  Or: launchctl load ~/Library/LaunchAgents/com.supersynergy.synapsed.plist"
fi

# Check syn CLI
if ! command -v syn >/dev/null 2>&1; then
  echo "⚠ syn CLI missing. Install: pip install synapse-memory"
  exit 1
fi

# Install hook scripts
cp "$(dirname "$0")/hooks/"*.sh "$HOOK_DIR/"
chmod +x "$HOOK_DIR"/*.sh

# Register in settings.json via python
python3 - <<PY
import json, os, sys
p = "$SETTINGS"
try:
    d = json.load(open(p))
except FileNotFoundError:
    d = {}
hooks = d.setdefault("hooks", {})

def add(event, matcher, cmd, timeout=5):
    arr = hooks.setdefault(event, [])
    for grp in arr:
        for h in grp.get("hooks", []):
            if h.get("command") == cmd:
                return
    arr.append({"matcher": matcher, "hooks":[{"type":"command","command":cmd,"timeout":timeout}]})

add("SessionStart", "startup", "$HOOK_DIR/session_start.sh", 5)
add("UserPromptSubmit", ".*", "$HOOK_DIR/prompt_submit.sh", 3)
add("Stop", ".*", "$HOOK_DIR/stop_extract.sh", 10)

json.dump(d, open(p,"w"), indent=2)
print("✓ hooks registered in", p)
PY

echo "✓ synapse-claude-code installed"
echo ""
echo "Hooks active:"
echo "  SessionStart     → inject top-8 project memories"
echo "  UserPromptSubmit → semantic memory context if prompt mentions project"
echo "  Stop             → auto-extract decisions from session tail"
