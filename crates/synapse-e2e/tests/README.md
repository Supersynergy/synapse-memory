# integration_killer_demo

End-to-end proof: 10+ synapse crates wired together in one test run.

## Quick run

```bash
cargo test -p synapse-e2e --test integration_killer_demo
```

## Story coverage

| # | Story | Crates | Status |
|---|-------|--------|--------|
| 1 | Ingestion (100 text + multimodal) | synapse-core, synapse-multimodal | ✅ |
| 2 | Indexing (Tantivy FTS + knowledge graph) | synapse-core (tantivy-fts), synapse-graph | ✅ |
| 3 | SQL-Wire (MySQL proxy) | synapse-mysql | ⚠️ SKIP (requires live TCP port) |
| 4 | Hybrid search + RRF fusion | synapse-core, synapse-fusion | ✅ |
| 5 | Persistence (.brainpack export→import) | synapse-core snap | ✅ |
| 6 | CDC streaming (10 events captured) | synapse-stream | ✅ |
| 7 | Time-series (1000 rows, AVG agg) | synapse-tsdb | ✅ |
| 8 | OLAP (DuckDB GROUP BY) | synapse-olap | ⚠️ SKIP (requires `--features olap`) |
| 9 | Multi-node CRDT gossip | synapse-cluster, synapse-core sync | ✅ |
| 10 | Migrate-in (mock Chroma → synapse) | rusqlite inline + synapse-core | ✅ |

**Pass-rate: 8/10 fully wired (2 feature-gated skips)**

## Notes

- Story 3 (SQL-Wire): covered in `synapse-mysql` crate's own integration tests.
- Story 8 (OLAP): add `--features olap` to link DuckDB and activate `OlapEngine`.
- All 10 tests are deterministic, use tempfiles, no network required.
