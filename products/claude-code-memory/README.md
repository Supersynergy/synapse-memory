# Synapse Claude Code Memory

Persistent, project-scoped memory for Claude Code, Codex-style local work, and
multi-session coding agents.

## Product Promise

Your coding agent remembers prior decisions, fixes, files, tools, and session
handoffs without dumping noisy history into every prompt.

## Included

- `UserPromptSubmit` and `SessionStart` hooks.
- Budgeted context packer with task-mode routing.
- `Stop` extraction for decisions and switches.
- Telepathy daemon that tails Claude JSONL sessions and writes compact scoped
  memories in batches.
- Fast recall path through `synx-fast`, with `synx` reserved for freshness.

## First Run From Monorepo

```bash
./install.sh --local
integrations/claude-code/install.sh
integrations/claude-code/telepathy/install.sh
synx-fast doctor
```

## Product Verify

```bash
products/claude-code-memory/scripts/verify.sh
```

## Buyer Angle

Sell this first to heavy Claude Code/Codex users. The magic moment is opening a
new session and seeing useful prior context without manual notes.
