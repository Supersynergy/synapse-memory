# Synapse Freshness Router

Version-aware context guard for AI coding.

## Product Promise

Stop agents from using stale package/API knowledge. Resolve local project
versions first, prefer local docs, then official remote docs, and only then fall
back to model training memory.

## Included

- Fresh context logic from the Claude hook.
- Prompt hook entrypoint.
- Quickstart docs showing the fast local agent mode.

## Current Product Shape

This is not yet a standalone binary. It is product-ready as part of the
Claude/Codex hook bundle, and can become standalone as `synx fresh-context` /
`synapse-fresh` later.

## Verify

```bash
products/freshness-router/scripts/verify.sh
```

## Key Rule

Freshness is evidence, not memory. Synapse should remember decisions and prior
work; the router should resolve current docs and versions at use time.
