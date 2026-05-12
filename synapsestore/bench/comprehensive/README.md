# Synapse Comprehensive Benchmark

## Turbo Daemon (required for Phase I / Phase D speedup)

The turbo daemon runs on `localhost:9477` and provides T1 pre-computed cache (~0.0003ms),
T2 NumPy SIMD search (~0.05ms), and T3 ONNX embedding (~3.2ms).
Without the daemon, Phase I falls back to direct sqlite-vec (~1.2x speedup instead of >100x).

### Start

```bash
launchctl start de.supersynergy.synapse-turbo
```

### Stop

```bash
launchctl stop de.supersynergy.synapse-turbo
```

### Status / Health

```bash
launchctl print gui/$(id -u)/de.supersynergy.synapse-turbo
python3 -c "import urllib.request,json; print(json.dumps(json.loads(urllib.request.urlopen('http://localhost:9477/health').read()), indent=2))"
```

Sample `/health` response:
```json
{
  "status": "ok",
  "uptime_s": 42.3,
  "pid": 12345,
  "cache_hits": 1000,
  "cache_misses": 1,
  "hitrate": 0.999,
  "precomputed_queries": 2,
  "numpy_docs": 161477,
  "embed_disabled": false
}
```

### Logs

- stdout: `~/.synapse/turbo-stdout.log`
- stderr: `~/.synapse/turbo-stderr.log`

### launchd plist

`~/Library/LaunchAgents/de.supersynergy.synapse-turbo.plist`

Key settings:
- `KeepAlive: true` — auto-restarts on crash
- `RunAtLoad: true` — starts on login
- `ThrottleInterval: 10` — minimum 10s between restarts (crash loop protection)
- `FASTEMBED_CACHE_DIR` — points to pre-downloaded ONNX model cache

### Root Cause of Previous Crashes

The old `com.synapse.turbo.plist` pointed to `~/.claude/synapse-turbo.py` which failed
to find `onnx/model.onnx` because the `FASTEMBED_CACHE_DIR` env var was not set.
This caused a crash loop every ~1s (crash → KeepAlive restart → crash again),
resetting daemon state and making Phase I fall back to direct sqlite-vec (1.2x instead of >100x).

Fix: new plist uses the correct script path + `FASTEMBED_CACHE_DIR` env var pointing to
`/Users/master/projects/synapse/.fastembed_cache` where models are pre-downloaded.

## Running the Benchmark

```bash
cd /Users/master/projects/synapse/bench/comprehensive
python3 bench.py --phases ABICDEF
```

Phase I output with daemon active:
```
first=N.NNms  mean_repeat=0.0003ms  speedup=XXXX.Xx  cache_hits_delta=1000  daemon_active=True
```
