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
