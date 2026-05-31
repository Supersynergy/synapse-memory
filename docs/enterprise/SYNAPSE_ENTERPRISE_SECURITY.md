# Synapse Enterprise Security Brief

## Executive Summary

Synapse is a local-first memory and freshness layer for coding agents. It is
designed for teams that want persistent agent context without sending private
code, decisions, or customer data to a managed memory cloud.

## Data Flow

1. Agent hooks observe prompts, tool events, edits, commits, and session stops.
2. The hook normalizes the event and attaches scope metadata such as project,
   agent, package, version, source URI, and lifecycle status.
3. Synapse stores the memory in a local SQLite-backed brain file and optional
   vector side tables.
4. Query-time recall is scope-first: project/session/package memories are
   filtered before global ranking.
5. Freshness context resolves local lockfiles and manifests before falling back
   to remote documentation or model training knowledge.

## Privacy Model

- Default deployment is local/on-prem.
- The daemon communicates over a Unix socket by default.
- API-key authentication is supported through `SYNAPSE_API_KEY`.
- Memories can carry provenance metadata: `source_uri`, `valid_from`,
  `valid_until`, `derived_from`, `supersedes`, and `contradicts`.
- PII-sensitive deployments can place DSGVO/PII masking before ingestion.

## Deletion And Retention

- Individual documents can be deleted by id through the daemon `Delete` op.
- Tenant/project scopes can be enumerated and deleted by metadata policy.
- Snapshots can be exported for retention or audit evidence.
- Recommended enterprise policy: retain raw hook events for 30-90 days, retain
  distilled decisions while valid, and mark superseded/stale facts rather than
  blindly returning them.

## Audit Evidence

Recommended audit fields:

- `scope`
- `agent_id`
- `project`
- `kind`
- `source_uri`
- `created_at`
- `valid_from`
- `valid_until`
- `confidence`
- `supersedes`
- `contradicts`

## On-Prem Deployment

Supported packaging paths:

- Single binary: `synx`, `synx-fast`, `synapsed`
- Docker image: `synapse-agentdb:local`
- Homebrew formula template: `packaging/homebrew/synapse.rb`
- Claude-Code hooks: `integrations/claude-code/install.sh`

## Security Checklist

- Set `SYNAPSE_API_KEY` for shared machines.
- Keep the brain file on encrypted disk.
- Restrict Unix socket permissions to the local user or agent group.
- Disable cloud doc adapters where policy requires full offline operation.
- Run `make bench-agent-memory` before rollout to capture local recall and
  latency baselines.

## Competitive Position

Synapse does not replace enterprise policy systems. Its enterprise edge is the
combination of local deployment, scoped recall, freshness/version guards,
observable benchmarks, and agent-workflow integrations.
