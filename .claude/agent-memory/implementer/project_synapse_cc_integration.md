---
name: Synapse Claude Code Integration 2026-05-12
description: Audit + fix of Synapse hooks, skills, triggers for Claude Code auto-recall/store
type: project
---

MCP wired in ~/.claude.json (synapse entry, --sock /tmp/synapse.sock). Binary at ~/.local/bin/synapse-mcp.

Hooks in settings.json (verified 2026-05-12):
- UserPromptSubmit: smart-context.py (has _inject_synapse_hits via socket)
- SessionStart: synapse_sessionstart.sh (async, hybrid recall on project name)
- PostToolUse: synapse_auto_store.sh (Write/Edit/git commit → synx put, 30s throttle)

New files created:
- ~/.claude/hooks/synapse-user-prompt.sh — standalone UserPromptSubmit hook (backup, not yet wired — smart-context.py covers it)
- ~/.claude/hooks/synapse-post-tool-store.sh — alternative PostToolUse (not wired — synapse_auto_store.sh covers it)
- ~/.claude/cts/skills/synapse-recall/SKILL.md — synapse recall skill with CLI usage + MCP tools

Index additions:
- ~/.claude/skills.idx: synapse-recall entry
- ~/.claude/skills-triggers.idx: 13 new triggers (recall, remember, find similar, vec search, etc.) → synapse-recall

**Why:** Hooks existed but weren't wired (UserPromptSubmit was empty, SessionStart missing synapse). Now all 3 hook events hit Synapse.

**How to apply:** Don't re-create these hooks. They exist and are wired.
