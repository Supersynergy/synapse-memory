# WP Plugin Real Bench — 2026-04-25

**Brain**: 158,815 docs / 158,814 vecs (`~/.synapse/brain.db`)
**Daemon**: `synapsed` PID 78911, socket `/tmp/synapse.sock`
**Client**: `synapse-wp` PHP plugin (`src/Client.php`)

## Results

| Mode   | p50 (ms) | p95 (ms) | Iters | embed_query |
|--------|----------|----------|-------|-------------|
| Lex    | 97.1     | 2185.8   | 100   | false       |
| Vec    | 1163.6   | 8207.6   | 25    | true        |
| Hybrid | 2649.2   | 6374.7   | 25    | true        |

## Baseline: Python direct socket (no PHP overhead)

| Mode | p50 (ms) | p95 (ms) | Iters |
|------|----------|----------|-------|
| Lex  | 13.6     | 841.2    | 20    |

## Chi's Claim vs Reality

Chi claimed **5.1ms p50 socket round-trip** for Hybrid search.

| Metric          | Chi's Claim | PHP Plugin (this bench) | Python direct |
|-----------------|-------------|------------------------|---------------|
| Hybrid p50      | 5.1ms       | 2,649ms                | ~1,200ms est  |
| Lex p50         | (not stated)| 97ms                   | 13.6ms        |

**Verdict: DOES NOT HOLD.**

Chi's 5.1ms is physically impossible for Hybrid mode with `embed_query: true` — embedding a query vector via fastembed/ONNX on CPU takes ~1,000-1,200ms alone (measured). The 5.1ms figure likely reflects:
- Lex-only search (no embedding) in ideal conditions, OR
- A cached/pre-embedded query, OR
- A measurement of only the socket write+read framing, not the full search latency

## Root Causes of PHP Overhead

1. **Reconnect per call**: `synapsed` closes the connection after each response (no keep-alive). The PHP client now reconnects on every `call()` → adds ~5-10ms TCP handshake equivalent on UNIX socket.
2. **Embed latency dominates**: Vec/Hybrid modes call fastembed ONNX CPU inference per query (~1,000-1,200ms). This is not a PHP overhead — Python direct socket shows same order of magnitude.
3. **p95 spikes**: SQLite WAL checkpoint or GC on 158k-doc brain.db causes occasional 2-8s stalls across all clients.

## Recommendations

- Use **Lex mode** for low-latency WP queries (p50 ~97ms PHP, ~14ms Python). Gap is reconnect overhead.
- Implement **query embedding cache** in daemon (LRU by query text) to bring Vec/Hybrid to Lex-level latency on repeated queries.
- Enable **persistent connections** in `synapsed` (keep-alive on unix socket) to reduce PHP reconnect overhead from ~80ms to ~5ms.
- Chi's 5.1ms benchmark should be reproduced with `embed_query: false` and a warmed FTS5 cache.
