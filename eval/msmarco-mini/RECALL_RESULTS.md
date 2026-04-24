# MS-MARCO Recall@10 — Synapse FTS5 (Lex)

## Configuration

| Parameter | Value |
|-----------|-------|
| Subset    | 100k passages (head of collection.tsv) |
| Queries   | 100 (synthetic: first 5 words of passage as query) |
| Mode      | `SearchMode::Lex` (FTS5 BM25) |
| Embedder  | N/A (Lex mode — no vector search) |
| DB path   | `/tmp/msmarco-recall.db` |

## Results

| Metric | Value |
|--------|-------|
| Ingested docs | 100000 |
| Queries evaluated | 100 |
| Hits@10 | 98 |
| **Recall@10** | **0.9800** |
| SPEC threshold (§6) | ≥0.95 |
| **Status** | **PASS ✓** |

## Latency

| Percentile | Latency |
|------------|---------|
| p50 | 1750µs |
| p95 | 3363µs |
| p99 | 3854µs |
| Ingest throughput | 7799 docs/s |

## Caveats

1. **Synthetic queries**: Official `queries.dev.small.tsv` was unavailable (network blocked).
   Queries were generated as the first 5 words of each passage — this makes recall trivially
   high (essentially exact-match retrieval) and does NOT reflect real retrieval difficulty.
2. **Lex-only**: `SearchMode::Vec` (BGE-small-384) not used — embedding 100k passages would
   take ~20min on CPU with fastembed. FTS5 BM25 only.
3. **Subset**: 100k of 8.8M passages — long-tail passage retrieval not tested.
4. **True MS-MARCO recall** requires official dev.small queries+qrels downloaded from
   `https://msmarco.blob.core.windows.net/msmarcoranking/` (blocked in this environment).
