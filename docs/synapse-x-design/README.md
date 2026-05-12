# Synapse-X Design

Design only. Not implemented.

Lives here because it extends `crates/synapse-market`. Adapter for winvestment in W9.

- **SYNAPSE-X-DESIGN.md** — full design doc (Hebelwort lens + omni-8-perspectives + ghmax-ranked choices + SuperML router + 8-week roadmap + bench-gates)
- **SYNAPSE-X-TICKDATA-ML.md** — tick-storage + L2-book delta + ML co-located (write-time feature DAG, online-learners in-page, conformal-as-column, zero-copy MLX). 100×+ targets per workload + 16w extended roadmap.
- **SYNAPSE-X-TESTING.md** — 8-layer test pyramid, differential-vs-DuckDB+SQLite, fuzz, soak, cross-lang ABI, ML-discipline, bench-as-test gates, bench-history DB.
- **Bench targets**: 10-100× vs DuckDB/Parquet/MariaDB on quant workloads
- **New modules**: `store/`, `series/`, `signal/`, `embed/`, `analytics/`, `router/`, `kg/`, `api/`
- **Reuses**: synapse-core, synapse-kernel (SimSIMD), synapse-metal, synapse-colbert, synapse-learn, synapse-mcp
- **New LoC estimate**: 3-5k Rust

Anchor: existing `synapse-market` already has Market/OHLCV/Regime/News/Backtest. v2 makes it the **quant store** beating DuckDB+Parquet on every realistic workload, embedded, single-binary, with auto-embed-on-insert.

Bridge to winvestment: replace better-sqlite3 hot-paths in W9 → expect 7-16× on /heatmap, /api/indicators, /insights.
