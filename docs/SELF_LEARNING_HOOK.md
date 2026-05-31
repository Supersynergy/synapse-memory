# Self-Learning Git Post-Commit Hook

## Install

```bash
bash scripts/install_hooks.sh
```

## What It Does

On every commit, logs to Synapse memory or `~/.synapse/dev-log.jsonl` (fallback):

```
commit:<hash> | files:<file1 file2 ...> | msg:<commit message>
```

## Fallback Behavior

- If `synx` CLI available: `synx put ... --tag synapse-dev`
- Else: appends JSON line to `~/.synapse/dev-log.jsonl`

## Reinstall After Clone

```bash
bash scripts/install_hooks.sh  # idempotent, safe to re-run
```
