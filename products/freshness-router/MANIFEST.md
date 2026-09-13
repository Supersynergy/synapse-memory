# Freshness Router Manifest

## Copied Release Files

- `files/hooks/synapse_context.py`
- `files/hooks/prompt_submit.sh`
- `files/QUICKSTART.md`
- `files/evidence-example.json`
- `files/primary-sources.json`
- `scripts/register-primary-sources.py`
- `scripts/render-obsidian-status.py`
- `scripts/refresh-knowledge.sh`

## Canonical Source Files

- `integrations/claude-code/hooks/synapse_context.py`
- `integrations/claude-code/hooks/prompt_submit.sh`
- `docs/QUICKSTART.md`

## Key Env

- `SYNAPSE_FRESH_NATIVE`
- `SYNAPSE_FRESH_NATIVE_TIMEOUT`
- `SYNAPSE_FRESH_NO_REGISTRY`
- `SYNAPSE_FRESH_TTL_SEC`
- `SYNX_FRESH_BIN`
- `SYNAPSE_BRAIN_DB`
- `KNOWLEDGE_FRESHNESS_NOTE`

## Installed Runtime

- `/Users/master/Library/LaunchAgents/com.supersynergy.synapse-knowledge-freshness.plist`
- six-hour source refresh; 900-second and 4-GB step guards
- `/Users/master/Documents/Obsidian Vault/03_Resources/Knowledge Freshness Control Plane.md`

## Closed In This Slice

- `synx fresh-context` exposes package/API context and JSON.
- `synx fresh-evidence` validates generic evidence, TTL, hash, and status.
- Official sources register into the existing corpus database.
- The Obsidian note is a projection, not another database.

## Remaining Boundaries

- Add a docs-version benchmark separate from memory recall.
- Add claim-level review/promotion UI; ingestion alone creates candidates.
- Expand the primary registry only after per-source verification.
