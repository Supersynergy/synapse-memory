# 📡 Synapse Telepathy — Cross-Session Memory for Claude Code

**All your parallel Claude Code sessions share one live brain.**

Session A sees what Session B just fixed, which file C is editing, what prompt D got two seconds ago. Nobody else has this.

## What it does

```
┌─ Session A (project-x) ──┐   ┌─ Session B (project-y) ──┐   ┌─ Session C (docs) ──┐
│  writes jsonl transcript │   │  writes jsonl transcript │   │  writes jsonl ...   │
└────────────┬─────────────┘   └────────────┬─────────────┘   └────────────┬────────┘
             │                               │                              │
             ▼                               ▼                              ▼
     ╔═══════════════════════ telepathy daemon (4s poll) ═══════════════════════╗
     ║  tail deltas → extract prompt/reply/tool → push to synapse via `syn put` ║
     ╚═══════════════════════════════════════╤═════════════════════════════════╝
                                             ▼
                                    ┌────────────────┐
                                    │ synapse brain  │
                                    │   FTS5 + vec   │
                                    └────────┬───────┘
                                             │ SessionStart / UserPromptSubmit hook
                                             ▼
                        ## 📡 Telepathy — recent from OTHER sessions
                        - [26e913a2][project-y][tools] Read,Edit,Bash
                        - [f8a3b712][docs][prompt] fixing auth middleware
                        - [91ccaa20][project-x][reply] added retry logic ...
```

## Benchmarks (M4 Max, Synapse 2.0)

| Metric | Value |
|---|---|
| End-to-end jsonl → visible in other session | **~3 s** (poll=4s) |
| `syn put` latency | 50 ms |
| `syn search` latency | 22 ms |
| Hook injection | 47 ms |
| Daemon RSS | 22 MB |
| Daemon CPU idle | ~0% (sleeps 4s between ticks) |
| Full tree scan (8676 jsonls) | 58 ms |

## Install

```bash
bash integrations/claude-code/telepathy/install.sh
```

Prereqs: `syn` CLI on PATH, Synapse daemon running (`synapsed -f ~/.synapse/brain.db &`).

## Verify

```bash
# live events arriving:
tail -f ~/.claude/telepathy/daemon.log

# current cross-session feed:
syn search telepathy | head

# daemon alive:
launchctl list | grep telepathy     # macOS
```

Open two Claude Code sessions in different cwds. Ask session A a question. Session B sees it in the next prompt's injected context.

## Tuning

Env vars (set in the plist or your shell):

| Var | Default | Meaning |
|---|---|---|
| `TELEPATHY_POLL` | `4.0` | seconds between ticks |
| `TELEPATHY_IDLE_CUTOFF` | `1800` | skip jsonls not modified within N seconds |
| `TELEPATHY_MAX_LINES` | `500` | max new lines scanned per file per tick |
| `SYN_BIN` | `syn` | path to Synapse CLI |

## What you can now do that nobody else can

- **Multi-session coding** — you're refactoring in one window while a review agent runs in another; each knows the other's changes.
- **Hand-off without reload** — kill a session, open a new one in the same repo, the new one already sees what the old one was doing.
- **Swarm debugging** — three sessions attack the same bug, each one's discoveries pooled.
- **Background pair** — let a loop-mode agent work on PR-review in a second window; your main session sees its findings as it files them.
- **Machine-wide coherence** — your laptop becomes one Claude, not N isolated Claudes.

## Uninstall

```bash
launchctl unload ~/Library/LaunchAgents/de.supersynergy.telepathy.plist
rm ~/Library/LaunchAgents/de.supersynergy.telepathy.plist
rm ~/.claude/scripts/telepathy_daemon.py ~/.claude/hooks/telepathy_inject.sh
rm -rf ~/.claude/telepathy
# then remove the two hook entries from ~/.claude/settings.json
```

## How it works (tech)

- **Extractor** reads user/assistant events from `~/.claude/projects/**/*.jsonl`, keeps only the first 240 chars of a prompt, 200 chars of a reply, or the list of tool names.
- **Tag format**: `[telepathy][<sid8>][<cwd>][prompt|reply|tools] <payload>` — so the hook can filter the current session via a cheap string match.
- **Baseline-skip**: on first sight of a file, the daemon records its size but emits nothing, so startup doesn't flood the brain with historical transcripts.
- **Idle cutoff**: files older than 30 min by mtime are skipped — full tree scan stays under 60 ms on 8k+ jsonls.
- **State**: one `~/.claude/telepathy/offsets.json` with `{path: byte_offset}`. Crash-safe, resume-safe.
