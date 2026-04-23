# synapse-claude-code — Auto-memory hook-pack

Three Claude Code hooks that give your agent persistent memory across sessions.

## What it does

| Hook | When | Action |
|---|---|---|
| `SessionStart` | Claude Code starts | Inject top-8 memories for `basename $PWD` |
| `UserPromptSubmit` | Every prompt | Inject top-3 semantic matches if score > 0.05 |
| `Stop` | Session ends | Extract decision lines, persist to synapse |

## Install

```bash
# prereq: daemon + python sdk
pip install synapse-memory
cargo install --git https://github.com/Supersynergy/synapse synapsed
synapsed -f ~/.synapse/brain.db &

# install hooks
git clone https://github.com/Supersynergy/synapse
cd synapse/integrations/claude-code
./install.sh
```

## Uninstall

```bash
# Remove hooks from ~/.claude/settings.json manually or:
jq 'del(.hooks.SessionStart,.hooks.UserPromptSubmit,.hooks.Stop)' \
   ~/.claude/settings.json > /tmp/s && mv /tmp/s ~/.claude/settings.json
rm -rf ~/.claude/hooks/synapse
```

## Config

Edit hook scripts in `~/.claude/hooks/synapse/`:
- `session_start.sh` — change `hybrid "$PROJ …" 8` to tune query/count
- `prompt_submit.sh` — adjust score threshold `>0.05`
- `stop_extract.sh` — edit grep regex for decision markers

## Bench

SessionStart hook runtime ~20ms (socket via syn). No perceivable delay.

## Why not MCP?

MCP adds stdio-JSON-RPC hop (~5ms). Direct socket via syn is the same latency with native shell ergonomics — and works even if MCP client not loaded.
