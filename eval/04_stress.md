# Stress Test Results

Platform: M4 Max, release build, daemon mode (`synapsed --lazy-embed`), no embeddings.
Docs: 20-word random vocabulary text per doc. Batches of 1,000–5,000.

## Insert + Search Scaling

| N docs | Insert time | Throughput | p50 search | p99 search | File size |
|--------|------------|-----------|-----------|-----------|-----------|
| 1,000 | 15ms | 68,639/s | 0.21ms | 0.40ms | 4KB |
| 11,000 | 157ms | 63,810/s | 1.84ms | 2.31ms | 2.6MB |
| 111,000 | 2,666ms | 37,512/s | 19.33ms | 21.29ms | 40MB |
| 1,111,000 | 41,042ms | 24,365/s | 234.73ms | 405.91ms | 388MB |

Note: N column shows cumulative docs in db (prior tests not cleared), so search degrades with db size.

## Key Findings

- Insert throughput drops ~3x from 1k to 1M (WAL write pressure, FTS5 triggers).
- Search at 1M docs: p50=235ms — significantly above the claimed `<20ms`. At 10k (realistic agent brain) p50=1.84ms, well within target.
- File size: ~350 bytes/doc at 1M scale. Zstd snap would compress ~3-5x.
- No OOM observed. RAM stable throughout (mmap + WAL).

## Recommendation

Synapse is designed for `<100k` docs per brain file. Beyond that, split into domain shards (one .db per domain/project). Claim of `<20ms` holds up to ~50k docs.
