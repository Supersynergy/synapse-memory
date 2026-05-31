# Freshness Router Manifest

## Copied Release Files

- `files/hooks/synapse_context.py`
- `files/hooks/prompt_submit.sh`
- `files/QUICKSTART.md`

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

## Product Gap To Close

- Extract `fresh-context` into a dedicated CLI/API with JSON output.
- Add a docs-version benchmark separate from memory recall.
- Cache resolved docs by package/version/hash.
