# Synapse Bench — Interim Results v2 (2026-04-25)

## Run Status

- **Full run PID**: 69879
- **Log**: `~/projects/synapse/bench/comprehensive/run_full_v2.log`
- **Profile**: full (100k docs, all phases A-J, ENGINE_TIMEOUT=1800s)
- **Engines**: sqlite-vec, duckdb, lancedb, chromadb, synapse, milvus, qdrant, pgvector
- **Status at T+90s**: DuckDB scale-curve running (1k/10k/100k insert sweep)
- **Skipped (cached)**: sqlite-vec, chromadb, synapse, qdrant (full results already exist)

## Phase A — Bulk Insert (ops/sec, 100k docs)

| Engine | ops/sec | RSS MB | Disk MB |
|--------|---------|--------|---------|
| sqlite-vec | 38,545 | 1,302 | 306 |
| synapse | 39,308 | 1,402 | 306 |
| chromadb | 764 | 1,836 | 494 |
| qdrant | — (timeout/error) | — | — |
| duckdb | running | — | — |
| lancedb | pending | — | — |
| milvus | pending | — | — |
| pgvector | pending | — | — |

## Phase J — Mixed Embed/Skip-Embed (NEW)

Phase J implemented and wired into `--phases=all`. Will appear in results once engines complete.

**Adapter classification:**
- **SynapseAdapter (daemon)**: probes `/put` endpoint for text-only insert; falls back to inline fastembed
- **SynapseAdapter (fallback)**: inline fastembed if available, else BLAS random projection simulation
- **All other adapters**: inline fastembed if available, else BLAS simulation (no native text-input API)
- `fastembed` package required for real embed timings; without it, overhead is simulated

## Dashboard

- **File**: `dashboard.html` (34KB, 4 Plotly charts)
- **Data**: 15 result rows from existing JSONL files
- **Charts**: Phase A ops/sec, latency comparison, resource usage, scale curve

## ETA Estimate

DuckDB at T+20s was inserting scale-curve rows. DuckDB with FLOAT[384] arrays is historically slow (~300-500 ops/s). At 100k docs × ~2ms/insert = ~200s for phase A alone. Full phases A-J for DuckDB: ~30-60 min. Remaining engines (lancedb, milvus, pgvector) should be faster. **Estimated total: 60-120 min.**

## Next Steps

- Monitor: `tail -f ~/projects/synapse/bench/comprehensive/run_full_v2.log`
- Re-run dashboard when engines complete: `python3 dashboard.py`
- Final report: `python3 report.py --partial`
