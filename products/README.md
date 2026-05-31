# Synapse Product Releases

This folder splits the current Synapse repo into product-ready release bundles.
Each product keeps a small copied `files/` payload plus a manifest pointing back
to the canonical source files in the monorepo.

## Product Matrix

| Product | Buyer | What It Ships | Verify |
|---|---|---|---|
| `agentdb-local` | AI engineers, local-agent power users | daemon, `synx-fast`, install, Docker, smoke path | `products/agentdb-local/scripts/verify.sh` |
| `claude-code-memory` | Claude Code/Codex/Cursor users | hooks, context packer, Telepathy multi-session memory | `products/claude-code-memory/scripts/verify.sh` |
| `freshness-router` | dev teams fighting stale AI API knowledge | lockfile/docs/training-data guard via hook context | `products/freshness-router/scripts/verify.sh` |
| `benchmark-kit` | buyers, skeptics, OSS reviewers | recall bakeoff, AgentDB public bench, three-session demo | `products/benchmark-kit/scripts/verify.sh` |
| `enterprise-onprem` | privacy/security-sensitive teams | Docker, security PDF, Homebrew/K8s/npm/deploy docs | `products/enterprise-onprem/scripts/verify.sh` |

## What Is Product-Ready Now

1. `agentdb-local` is the strongest standalone product. It has a clear CLI,
   daemon, installer, Docker path, and real smoke test.
2. `claude-code-memory` is the strongest wedge. It gives an immediate agent
   workflow upgrade without asking users to understand databases.
3. `benchmark-kit` is the proof product. It makes the performance claims
   reproducible and turns skepticism into a local command.
4. `enterprise-onprem` is sales collateral plus deploy packaging. It needs a
   harder security review before aggressive enterprise sales.
5. `freshness-router` is valuable but should stay paired with the hooks until it
   has a standalone CLI/API.

## Release Rule

Keep source of truth in the monorepo. The product folders are curated release
surfaces: docs, install commands, copied essential scripts, and verification.
When a source file changes, refresh the matching `files/` payload before release.
