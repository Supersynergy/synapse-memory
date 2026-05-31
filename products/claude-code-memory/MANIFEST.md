# Claude Code Memory Manifest

## Copied Release Files

- `files/install.sh`
- `files/hooks/prompt_submit.sh`
- `files/hooks/session_start.sh`
- `files/hooks/stop_extract.sh`
- `files/hooks/synapse_context.py`
- `files/hooks/test_synapse_context.py`
- `files/telepathy/daemon.py`
- `files/telepathy/install.sh`
- `files/telepathy/inject.sh`
- `files/telepathy/de.supersynergy.telepathy.plist`
- `files/telepathy/README.md`

## Canonical Source Files

- `integrations/claude-code/install.sh`
- `integrations/claude-code/hooks/synapse_context.py`
- `integrations/claude-code/hooks/prompt_submit.sh`
- `integrations/claude-code/hooks/session_start.sh`
- `integrations/claude-code/hooks/stop_extract.sh`
- `integrations/claude-code/telepathy/daemon.py`
- `integrations/claude-code/telepathy/install.sh`
- `integrations/claude-code/telepathy/inject.sh`

## Key Env

- `SYNX_FAST_BIN`
- `SYNX_FRESH_BIN`
- `SYNAPSE_SOCK`
- `SYNAPSE_SCOPE_KEY`
- `SYNAPSE_CONTEXT_MAX_PERSPECTIVES`
- `SYNAPSE_CONTEXT_TIMEOUT`
- `TELEPATHY_POLL`
- `TELEPATHY_BATCH`
- `TELEPATHY_SCOPE`
- `TELEPATHY_NO_SCOPE`

## Release Commands

```bash
python3 -m py_compile files/hooks/synapse_context.py files/telepathy/daemon.py
PYTHONPATH=sdk/python python3 -m pytest -q integrations/claude-code/hooks/test_synapse_context.py
```
