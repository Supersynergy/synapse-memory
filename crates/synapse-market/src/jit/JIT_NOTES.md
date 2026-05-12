# JIT Module — Notes

**Status**: LIVE. BlockArg API fixed, cranelift-native wired. 22 lib tests green.

## What was blocked (resolved)

cranelift 0.131 changed `InstBuilder::jump` to require `BlockArg` not `Value`.
Fixed: all jump/brif call sites updated to `BlockArg::Value(v)`.

## Architecture

- `predicate.rs` — `Col / Op / Predicate` AST + `Hash` impl
- `compile.rs`  — single-pass Cranelift codegen → `CompiledFn` (fn ptr + module lifetime)
- `mod.rs`      — `FilterCache`: predicate-hash → `CompiledFn` map

Compiled signature:
```
fn(ts, open, high, low, close, volume: *const, n: usize, out_mask: *mut u8) -> usize
```
Returns match count; writes 1/0 per bar into `out_mask`.

## Bench numbers (220 tickers × 2880 bars = 633,600 rows, M4 Max)

Run `cargo bench --bench jit_filter` to reproduce.

| backend       | p50 median   | vs naive    | notes                              |
|---------------|--------------|-------------|------------------------------------|
| naive Rust    | **449 µs**   | 1×          | branch-predictor friendly          |
| JIT cached    | **496 µs**   | 1.1× slower | cache-hit path, Cranelift overhead |
| DuckDB        | **807 µs**   | 1.8× slower | vectorized SQL, in-memory          |
| SQLite WAL    | **13.7 ms**  | 30× slower  | row engine, query parse overhead   |

Compile overhead p50: **< 5 ms** (10-trial median). JIT wins appear when:
- Many distinct predicates → single compile, N scans
- Dynamic predicate composition at runtime (not ahead-of-time Rust)
- SQLite/DuckDB query engine overhead dominates (< ~1M rows, JIT is ~equivalent)

## Alternatives considered (still valid escape hatches)

- `inkwell` (LLVM): heavier (+150MB), stable API 2yr+
- `dynasm-rs`: direct asm emit, smallest dep
- `wasmtime` interpreter: portable, no compile overhead, no JIT latency
