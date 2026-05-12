# synapse-market Release Checklist

## v0.2.0

Run all checks from the crate root (`crates/synapse-market/`).

### Pre-release gates

```
[ ] cargo test -p synapse-market RUSTC_WRAPPER=""   → 106 pass, 3 ignored
[ ] cargo bench -p synapse-market RUSTC_WRAPPER=""  → all gates pass
[ ] bash scripts/bench_compare.sh                   → 0 regressions
[ ] cargo clippy -p synapse-market -- -D warnings   → 0 errors
[ ] CHANGELOG updated                               → entry for v0.2.0
[ ] README badge URLs                               → green
[ ] cargo doc --no-deps -p synapse-market           → no broken intra-doc-links
```

### Full pipeline (optional, ~10min)

```
[ ] bash scripts/test_all.sh                        → "All gates passed"
```

### Checklist commands (copy-paste)

```bash
# Tests
RUSTC_WRAPPER="" cargo test -p synapse-market --no-fail-fast

# Benchmarks
RUSTC_WRAPPER="" cargo bench -p synapse-market

# Bench regression compare (uses cached bench output if present)
bash crates/synapse-market/scripts/bench_compare.sh

# Clippy
cargo clippy -p synapse-market -- -D warnings

# Docs
cargo doc --no-deps -p synapse-market

# Full pipeline
bash crates/synapse-market/scripts/test_all.sh
```

### Notes

- `RUSTC_WRAPPER=""` disables sccache for deterministic benches on M4 Max.
- Bench baselines: `crates/synapse-market/bench-baselines.toml`
- Bench history DB: `~/.synapse-x/bench-history.db`
- Record a bench run: `python3 crates/synapse-market/scripts/bench_history.py ingest bench-amx.txt --bench amx_minimal`
- Trend report: `python3 crates/synapse-market/scripts/bench_history.py --report`
