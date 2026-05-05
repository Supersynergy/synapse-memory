# Known Issues

## synapse-license — test mutex poisoning (non-blocking)

**Severity**: Low (test-only, not runtime)
**Crate**: `crates/synapse-license`
**Symptom**: 5 tests fail with `PoisonError` when run in parallel via `cargo test --workspace`.

```
thread 'tests::tampered_jwt_rejected' panicked at ... PoisonError { .. }
thread 'tests::valid_license_verifies' panicked at ... PoisonError { .. }
thread 'tests::tampered_cache_grace_fails_closed' panicked at ... PoisonError { .. }
```

**Cause**: Tests share a global `Mutex`-protected state. When one test panics first, the mutex becomes poisoned and all subsequent tests that lock it also fail.

**Workaround**: Run license tests in isolation: `cargo test -p synapse-license -- --test-threads=1`

**Fix planned**: Refactor tests to use per-test state (no global mutex).

---

## LongMemEval R@5 below target

**Severity**: Medium (quality metric)
**Metric**: R@5 = 0.30 (target: ≥ 0.85)
**Cause**: Per-message chunking at 76k chunks exposes Python↔Rust FFI overhead; cross-encoder reranker (synapse-rerank) not yet wired into the eval pipeline.
**Fix planned**: Phase P1 — wire `synapse-rerank` ONNX cross-encoder into the LongMemEval runner.

---

## synapse-ann / synapse-wal / synapse-seg — stub crates

**Severity**: Low (no functionality blocked)
**Crates**: `synapse-ann`, `synapse-wal`, `synapse-seg`
**Status**: Scaffolded with TODO markers. Scale-100M and crash-safe ingest paths not yet implemented.

---

## Corrected Marketing Claims (Day-13 audit 2026-04-25)

See [CORRECTIVE-ACTION-PLAN-2026-04-25.md](CORRECTIVE-ACTION-PLAN-2026-04-25.md) for full audit log.

| Claim | Was | Corrected to |
|-------|-----|-------------|
| WP speedup | "100× MySQL / 100× WP" | "5-10× WP overall; 30-100× search-only at 50k+ posts" |
| WP search | "187× WP search" | "50× LIKE→FTS5 + 8× FTS5→semantic + 1.2× overhead ≈ 480× compounded at 50k+ posts; vanilla LIKE wins at <1k posts" |
| Recall | "Recall 1.000 always" | "Recall 1.000 dense / ≥0.95 quantized (int8/binary)" |
| Library latency | "5µs reads" | "<50µs at 100k+ docs; daemon mode wins below 100k (WAL+mmap warm cache)" |
| Not shipped | synapse-wasm, synapse-mobile | Removed from claims |
| Customers | "enterprise pilot conversations" | No real customers; target is "paying design-partners"
