# WP Plugin Keep-Alive Fix — 2026-04-25

**Brain**: 158,815 docs / 158,814 vecs (`~/.synapse/brain.db`)
**Daemon**: `synapsed` (rebuilt + restarted via launchd `com.supersynergy.synapsed`)
**Socket**: `/tmp/synapse.sock`
**Client**: `synapse-wp` PHP plugin (`src/Client.php`) — persistent-socket lazy-singleton restored

## Root Cause (corrected)

The original bench report assumed `synapsed` closed the connection after each
response. Inspection of `crates/synapsed/src/main.rs::handle_conn` showed it
already loops on length-prefixed frames — the daemon was always keep-alive-capable.

The actual regression was **client-side**: `Client.php::callOnce` had been
patched to force-close `$this->fp` and re-`connect()` on **every** call,
costing ~80ms per round-trip on UNIX socket (TCP-handshake-equivalent +
PHP stream setup).

## Fixes Applied

1. **Daemon (`crates/synapsed/src/main.rs`)** — added defensive 60s idle timeout
   on the per-connection read loop using `tokio::time::timeout`. Prevents fd
   leaks from stale keep-alive sockets while preserving sub-ms reuse for
   back-to-back calls. (No protocol change; PHP framing already matched.)

2. **PHP client (`synapse-wp/src/Client.php`)** — restored persistent-socket
   lazy-singleton: `callOnce` now reuses `$this->fp`, only reconnects on I/O
   error, and retries once. Also hardened the header read to loop until 4
   bytes arrive (handles partial reads on warm sockets) and added a body-length
   guard.

## Results — Before / After

Same workload as `wp-plugin-real-bench.md`: 100 iters Lex, 50 iters Vec, 50 iters Hybrid.

| Mode   | Before p50 | **After p50** | Before p95 | **After p95** | Speedup p50 |
|--------|------------|---------------|------------|---------------|-------------|
| Lex    | 97.1 ms    | **4.5 ms**    | 2185.8 ms  | **16.6 ms**   | **21.6×**   |
| Vec    | 1163.6 ms  | **81.9 ms**   | 8207.6 ms  | **174.3 ms**  | **14.2×**   |
| Hybrid | 2649.2 ms  | **56.4 ms**   | 6374.7 ms  | **64.8 ms**   | **47.0×**   |

### Lex vs. Python direct baseline

| Client                       | Lex p50  |
|------------------------------|----------|
| Python direct (reconnect/call, prev report) | 13.6 ms |
| **PHP plugin, keep-alive (this fix)**       | **4.5 ms** |

PHP now beats the Python baseline because the Python test in the prior run
also reconnected per call. With persistent socket, framing overhead is the
only PHP-specific cost (~0.5ms on top of the daemon's ~4ms FTS5 query).

### Vec / Hybrid note

The previous report's Vec/Hybrid p50 (1.1–2.6s) reflected a **cold embedder**
plus reconnect-per-call. Once the BGE-small-en-v1.5 model is warm in the
daemon (`--lazy-embed` triggered on first query), per-query embedding cost
drops to the 50–80 ms range observed here. Embedding remains the dominant
cost for Vec/Hybrid and is out of scope for this fix.

## Files Changed

- `crates/synapsed/src/main.rs` — idle-timeout wrap on read loop
- `synapse-wp/src/Client.php` — persistent-socket restore + read hardening

## Reproduce

```bash
cargo build --release -p synapsed
cp target/release/synapsed ~/.local/bin/synapsed
launchctl kickstart -k gui/$(id -u)/com.supersynergy.synapsed
php /tmp/wp_bench.php   # see /tmp/wp_bench.php in this session
```
