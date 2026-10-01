# Synapse-Memory — project CLAUDE.md

Local-first **Context OS** for AI agents: bounded, cited, freshness-aware context with feedback. One SQLite-backed local brain — no Docker, no cloud. CLI + daemon + MCP tooling. Core promise: **best context, not biggest context.**

- Crate: `synapse-core` (crates.io) · MIT · Repo: https://github.com/Supersynergy/synapse-memory
- Local: `~/BASE/projects/synapse-memory` · current branch: **`main`** (split-memory gemergt 2026-07)
- Runtime socket: `/tmp/synapse.sock` :9477 (SQLite fallback `~/.synapse/brain.db`)

## Stack
Rust workspace (edition 2024). SimSIMD kernels (1-bit 71× / int8 46× / MRL-128 35× / f16 4×). MRL/f16/Hamming vectors. Members in `crates/*` (py, js, metal, embed-gpu, colbert, splade) + vendored `vendor/synapse-db`.

## Commands (just)
```bash
just check        # default gate (fmt + clippy + check)
just test         # cargo nextest
just ci           # check + test          ← full gate
just build
just check-layers # architecture/layer boundary check
```
Direct: `cargo clippy --all-targets --all-features -- -D warnings` · `cargo nextest run`.

## Quality / security
`deny.toml` (cargo-deny) · `audit.toml` (cargo-audit) · `clippy.toml`. Run before release. `ARCHITECTURE.md` = layer map; `CONTRIBUTING.md` = workflow.

## Lean-code notes
Vendored code (`vendor/synapse-db`) MUST carry provenance (upstream URL + commit SHA + sync-date) and stay visible to `cargo audit` — invisible copies are un-patchable CVEs (xz/CVE-2024-3094 era). Correctness-critical surface (vectors, SQLite, parsers) → KEEP maintained deps, never self-roll. Apply `/leancode` before adding crates.

## Release flow
`CHANGELOG.md` newest-first (currently `Unreleased`). Tag auf `main` schneiden vor Release-Claim.

Inherits global rules `~/.claude/CLAUDE.md` + workspace `~/BASE/projects/CLAUDE.md`.
<!-- BEGIN EVIDENCE-LOOP v:1 -->
## Evidence-Loop (bd × kundenradar)

- Work kommt aus `bd ready`; Claim erst mit Oracle (`--acceptance` oder
  `## Acceptance Criteria`/`oracle:` in der Description). `bd-evidence-gate`
  läuft auf `bd update --claim`/`bd close`: deklarierte `kr:`/`EV-`-Refs
  müssen in `~/BASE/projects/kundenradar/questions.db` resolvieren, sonst Block.
- Produkt-Requirements via `kr2bd <kr-id> "<title>" --dod "..." --oracle
  "<cmd>"` anlegen (external_ref + evidence_ids automatisch). Unzitierte
  Annahmen sind keine Requirements.
- Done = Oracle grün + Fresh-Context-Review. Post-Ship: Kundensignal nach
  ~14d erneut prüfen; persistiert es, reopen statt „shipped".
- Detail: `/Users/master/.claude/policies/evidence-loop.md`. Escape:
  `EVIDENCE_GATE=off`.
<!-- END EVIDENCE-LOOP -->

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:1105d646 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/core-concepts/sync-concepts.md for details and anti-patterns.

## Agent Context Profiles

The managed Beads block is task-tracking guidance, not permission to override repository, user, or orchestrator instructions.

- **Conservative (default)**: Use `bd` for task tracking. Do not run git commits, git pushes, or Dolt remote sync unless explicitly asked. At handoff, report changed files, validation, and suggested next commands.
- **Minimal**: Keep tool instruction files as pointers to `bd prime`; use the same conservative git policy unless active instructions say otherwise.
- **Team-maintainer**: Only when the repository explicitly opts in, agents may close beads, run quality gates, commit, and push as part of session close. A current "do not commit" or "do not push" instruction still wins.

## Session Completion

This protocol applies when ending a Beads implementation workflow. It is subordinate to explicit user, repository, and orchestrator instructions.

1. **File issues for remaining work** - Create beads for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **Handle git/sync by active profile**:
   ```bash
   # Conservative/minimal/default: report status and proposed commands; wait for approval.
   git status

   # Team-maintainer opt-in only, unless current instructions forbid it:
   git pull --rebase
   git push
   git status
   ```
5. **Hand off** - Summarize changes, validation, issue status, and any blocked sync/commit/push step

**Critical rules:**
- Explicit user or orchestrator instructions override this Beads block.
- Do not commit or push without clear authority from the active profile or the current user request.
- If a required sync or push is blocked, stop and report the exact command and error.
<!-- END BEADS INTEGRATION -->
