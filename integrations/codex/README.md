# Codex crash-safe resume

Codex already appends the live task transcript locally. This integration adds
the missing compact recovery layer: a content-minimal checkpoint before and
after mutating tools, plus a resume hint on the next session start.

## Install

```bash
python3 integrations/codex/install.py --dry-run
python3 integrations/codex/install.py install
```

Restart Codex after installation so it reloads `~/.codex/hooks.json`.

## What survives a disconnect

- session/thread id and working directory
- last hook event and tool name
- command verb plus command hash, never command arguments
- Git HEAD and changed path names, never file contents or tool output
- an append-only JSONL journal plus an atomically replaced latest snapshot

State lives under `~/.synapse/checkpoints/` with mode `0600`. Every append and
latest snapshot is `fsync`-ed. `SessionStart` injects an unfinished checkpoint
only when it is at most seven days old. It tells the next agent to inspect
current files, Git, and processes before repeating any mutation.

## Remove

```bash
python3 integrations/codex/install.py uninstall
```

The installer backs up an existing `hooks.json`, preserves unrelated hooks,
and de-duplicates its own entries.
