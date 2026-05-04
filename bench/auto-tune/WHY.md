# Synapse Auto-Tune — Best Config

**Source**: `results.jsonl` (30 configs)

## Best Config

```json
{
  "cache_size_mb": 1024,
  "mmap_size_mb": 1024,
  "page_size": 8192,
  "journal_mode": "WAL",
  "synchronous": "OFF",
  "batch_size": 10000,
  "insert_ops_s": 255048.7,
  "query_p50_ms": 0.005,
  "db_size_kb": 8.0
}
```

## Feature Importance (by impact on insert_ops_s)

| Rank | Feature | Importance |
|------|---------|-----------|
| 1 | `batch_size` | 209567.98 |
| 2 | `synchronous` | 74764.88 |
| 3 | `page_size` | 55894.21 |
| 4 | `journal_mode` | 38902.2 |
| 5 | `mmap_size_mb` | 31990.37 |
| 6 | `cache_size_mb` | 25513.6 |

## Key Takeaways

- **`batch_size`** is the highest-impact knob.
- Run `harness.py --full` for the exhaustive 432-point grid.
- Re-run `tune.py` after expanding dataset or changing workload.
