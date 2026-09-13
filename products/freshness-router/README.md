# Synapse Freshness Router

Source-backed freshness guard for agents.

## Product Promise

Reduce the post-training knowledge gap for configured domains. Resolve local
project versions first, check official sources, preserve hashes and timestamps,
and expose stale or blocked evidence instead of falling back silently to model
memory.

## Included

- Fresh context logic from the Claude hook.
- Prompt hook entrypoint.
- Quickstart docs showing the fast local agent mode.
- Generic JSON evidence contract for non-package facts.
- Primary-source watch registry backed by the existing Synapse corpus database.
- Obsidian status renderer and a bounded daily refresh entrypoint.

## Current Product Shape

The package/API guard is available as `synx fresh-context`. Generic evidence is
checked with `synx fresh-evidence`. Recurring source ingestion uses `synx
corpus`; no second database is created.

## Evidence Contract

Required fields: `id`, `source_uri`, `source_kind`, `observed_at`,
`content_hash`, `ttl_sec`, and `status`. Optional provenance fields are
`published_at`, `valid_from`, `review_verdict`, and `supersedes`.

Statuses are `candidate`, `verified`, `stale`, `superseded`, and `blocked`.
Only unexpired `verified` evidence is current. A verified record needs a review
verdict. Hashes use `blake3:<64 hex characters>`. Observations before 2024-06-01
UTC are rejected by default.

```bash
synx fresh-evidence \
  products/freshness-router/files/evidence-example.json \
  --require-current
```

## Primary Sources

Preview registration:

```bash
python3 products/freshness-router/scripts/register-primary-sources.py
```

Apply and refresh once:

```bash
products/freshness-router/scripts/refresh-knowledge.sh
```

`files/primary-sources.json` contains official changelogs, specifications, and
release pages. ghmax remains a discovery signal; it does not promote a source
to `verified`. The installed launch agent runs every six hours; Freshdocs does
a full refresh only when its oldest cache class reaches three days.

The Obsidian projection also reports the read-only Primequellen market/macro
store, including failed and standby source counts. Primequellen remains a
domain evidence store; it is not treated as proof for unrelated claims.

## Verify

```bash
products/freshness-router/scripts/verify.sh
```

## Key Rule

Freshness is evidence, not memory. Synapse should remember decisions and prior
work; the router should resolve current docs and versions at use time.

No finite source registry can make an agent omniscient. The operational oracle
is narrower: every configured volatile claim must be fresh, cited, hashed, or
explicitly stale/blocked before it is used.
