# Smoke Test Results

## Build

```
cargo build --release  # 29.25s cold, 0 errors, 1 warning (unused field in synapse-mcp)
```

All 4 crates compiled: synapse-core, synapse-cli, synapsed, synapse-mcp.

## Tests

```
cargo nextest run
5/5 PASS in 0.043s
  db::tests::open_migrate_put_lex
  db::tests::dedup_same_text
  db::tests::vec_search
  db::tests::hybrid_search
  snap::tests::roundtrip
```

## Daemon Smoke

```
synapsed -f /tmp/synapse-eval/bench.db -s /tmp/synapse-eval.sock --lazy-embed
ping → Pong  (RPC OK)
```

## Bench (1k docs, daemon mode, no embeddings)

```
put_batch 1000: 18.1ms  (55,212 docs/s)
lex search p50=0.24ms  p95=0.66ms
```

Claimed benchmarks: 15.9ms insert / 0.31ms lex. Measured: 18.1ms / 0.24ms. Within margin.

## Bench Script

`bench/bench_extended.sh` exists. Not run (requires Python clients and competing stores installed).
