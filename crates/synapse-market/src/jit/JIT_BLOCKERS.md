# JIT Module — Blocked

**Status**: scaffolded (mod.rs + predicate.rs + compile.rs) but excluded from lib.rs build.

## Root cause

cranelift 0.131 changed `InstBuilder::jump` signature:
- Old: `fn jump(self, block: Block, args: impl IntoIterator<Item = &Value>)`
- New: `fn jump(self, block: Block, args: impl IntoIterator<Item = &BlockArg>)`

Our compile.rs:116 still passes `&[Value]`, hence E0271.

## Fix path (15-30min when picked up)

1. Replace `b.ins().jump(loop_hdr, &[next_i, new_count])` with
   `b.ins().jump(loop_hdr, &[BlockArg::Value(next_i), BlockArg::Value(new_count)])`
2. Same for `&[i0, count0]`
3. Verify `block_param` API hasn't moved (was `BlockArg::Value` exposing)

## Why bailed now

- Stream-watchdog stalled the agent at 600s mid-fix
- Library compile-time impact of full cranelift = +60s per build (slows iteration)
- Filter-scan workload currently fine with naive Rust loop until proven bottleneck

## Re-enable

```rust
// lib.rs
pub mod jit;
```

After fixing the 2 jump-call sites.

## Alternative if cranelift churn keeps biting

- Use `inkwell` (LLVM-bind) — heavier (+150MB build), but stable API since 2 years
- Use `dynasm-rs` — direct asm emit, smaller dep, stable
- Use `wasmtime` interpreter — no JIT but portable, no compile-overhead
- Use `regex-automata` for predicate-as-DFA — works for `cmp Op` chains, no AND/OR
