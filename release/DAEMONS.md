# Synapse Daemons

Two launchd agents for production Synapse operation.

## Install

```bash
chmod +x release/install-daemons.sh
./release/install-daemons.sh
```

Builds release binaries, copies to `/usr/local/bin/`, installs plists to `~/Library/LaunchAgents/`, bootstraps both agents.

## Agents

| Agent | Binary | Schedule | Log |
|-------|--------|----------|-----|
| `com.supersynergy.synapse.extract` | `synapse-extract-worker` | continuous 30s loop, KeepAlive | `~/Library/Logs/synapse-extract.log` |
| `com.supersynergy.synapse.lifecycle` | `synapse-lifecycle` | nightly 03:30 | `~/Library/Logs/synapse-lifecycle.log` |

## Environment Variables

| Var | Default | Effect |
|-----|---------|--------|
| `MINIMAX_API_KEY` | — | If set, worker auto-selects MiniMax extractor |
| `MINIMAX_BASE_URL` | `https://api.minimax.chat/v1` | MiniMax endpoint |
| `MINIMAX_MODEL` | `MiniMax-Text-01` | Model ID |
| `RUST_LOG` | `info` | Log level |

## CLI Options

```
synapse-extract-worker --db ~/.synapse/brain.db --batch 16 --interval-ms 30000 --extractor auto|rule|minimax|mlx
synapse-extract-worker --once   # single pass + exit

synapse-lifecycle --db ~/.synapse/brain.db --jaccard 0.7 --max-rows 5000 --decay-half-life-days 30
```

## Troubleshooting

**Uninstall:**
```bash
launchctl bootout gui/$(id -u)/com.supersynergy.synapse.extract
launchctl bootout gui/$(id -u)/com.supersynergy.synapse.lifecycle
rm ~/Library/LaunchAgents/com.supersynergy.synapse.{extract,lifecycle}.plist
```

**Logs:** `tail -f ~/Library/Logs/synapse-{extract,lifecycle}.log`

**Status:** `launchctl print gui/$(id -u)/com.supersynergy.synapse.extract`
