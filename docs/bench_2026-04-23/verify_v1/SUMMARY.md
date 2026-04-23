# Synapse Verification — 20 Use-Cases × 8 Engines

**Date**: 2026-04-23 · **Host**: M4 Max 128GB · **N**=10000 · **Q**=200 · **runs**=3 (median)
**Harness**: `bench/bench_verify_v1.py` · **Raw**: `docs/bench_2026-04-23/verify_v1/`

## Winner per UC (lower ms = better; only ok rows)

| UC | #1 | #2 | #3 |
|---|---|---|---|
| UC01_bulk_ingest | SQLite FTS5 (36.532ms) | Synapse v2 (67.000ms) | LanceDB (80.131ms) |
| UC02_stream_ingest | — | — | — |
| UC03_bm25_query | Synapse v2 (0.009ms) | sqlite-vec (0.015ms) | SQLite FTS5 (0.018ms) |
| UC04_vec_query | Synapse v2 (0.022ms) | Chroma (0.582ms) | Meilisearch (1.721ms) |
| UC05_hybrid | Synapse v2 (0.058ms) | Meilisearch (2.003ms) | sqlite-vec (5.849ms) |
| UC06_kg_3hop | Synapse v2 (2.210ms) | — | — |
| UC07_temporal_filter | SQLite FTS5 (0.024ms) | Synapse v2 (0.110ms) | — |
| UC08_meta_vec | Synapse v2 (0.350ms) | Chroma (4.882ms) | LanceDB (5.482ms) |
| UC09_knn_k1000 | — | — | — |
| UC09_knn_scales | Synapse v2 (0.022ms) | Chroma (0.531ms) | LanceDB (3.942ms) |
| UC10_update_10k | sqlite-vec (2707.354ms) | — | — |
| UC11_delete_compact | sqlite-vec (400.551ms) | — | — |
| UC12_cold_start | SQLite FTS5 (0.224ms) | Synapse v2 (0.790ms) | sqlite-vec (2.302ms) |
| UC13_concurrent_read | — | — | — |
| UC14_concurrent_write | — | — | — |
| UC15_rss_peak | — | — | — |
| UC16_disk_mb | — | — | — |
| UC17_recall10 | — | — | — |
| UC18_multilingual | SQLite FTS5 (0.026ms) | — | — |
| UC19_recovery | — | — | — |
| UC20_lib_api | Synapse v2 (0.015ms) | sqlite-vec (3.679ms) | — |

## Synapse Position

- Total UCs with at least one measurement: **13**
- Synapse measurable (ok=True): **11** / 20
- Synapse #1 rank: **7** / 13 (54%)
- Synapse top-3 rank: **10** / 13 (77%)