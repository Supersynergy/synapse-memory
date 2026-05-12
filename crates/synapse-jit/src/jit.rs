//! Cranelift-JIT compilation of QueryPlan → native code.
//!
//! Compiled function signature (C ABI):
//!   fn query(rows_ptr: *const i64, ncols: i64, nrows: i64,
//!            out_ptr: *mut i64, out_len_ptr: *mut i64) -> ()
//!
//! Each row is `ncols` consecutive i64 values.
//! The compiled function iterates all rows, applies filter+projection,
//! and writes matching rows into out_ptr (caller allocates ncols*nrows i64s).
//! out_len_ptr receives the number of output rows.

use std::collections::HashMap;

use anyhow::{Context, Result};
use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};

use crate::{
    ir::{CmpOp, Expr, GroupBySumPlan, HashJoinPlan, QueryPlan},
    schema::Schema,
    Row,
};

// ── extern-C callbacks for GROUP BY / HASH JOIN ──────────────────────────────
// These are called from JIT-compiled code via C ABI.

/// Called from JIT inner loop: accumulate key→sum into a flat bucket array.
/// bucket_ptr: *mut i64 array of [key0, sum0, key1, sum1, ...] (n_buckets pairs, -1 = empty)
/// n_buckets: total bucket count (power-of-2)
#[no_mangle]
pub unsafe extern "C" fn jit_gb_accumulate(
    bucket_ptr: *mut i64,
    n_buckets: i64,
    key: i64,
    val: i64,
) {
    let mask = (n_buckets - 1) as usize;
    let buckets = std::slice::from_raw_parts_mut(bucket_ptr, (n_buckets * 2) as usize);
    let mut h = (key as usize).wrapping_mul(0x9e3779b97f4a7c15);
    loop {
        let slot = (h & mask) * 2;
        let k = buckets[slot];
        if k == i64::MIN {
            // empty
            buckets[slot] = key;
            buckets[slot + 1] = val;
            return;
        } else if k == key {
            buckets[slot + 1] += val;
            return;
        }
        h = h.wrapping_add(1);
    }
}

/// Called from JIT inner loop: probe hash set for join. Returns 1 if key found, 0 otherwise.
/// set_ptr: *const i64 flat open-address hash set (i64::MIN = empty)
#[no_mangle]
pub unsafe extern "C" fn jit_hj_probe(set_ptr: *const i64, n_buckets: i64, key: i64) -> i64 {
    let mask = (n_buckets - 1) as usize;
    let buckets = std::slice::from_raw_parts(set_ptr, n_buckets as usize);
    let mut h = (key as usize).wrapping_mul(0x9e3779b97f4a7c15);
    loop {
        let slot = h & mask;
        let k = buckets[slot];
        if k == i64::MIN {
            return 0;
        } else if k == key {
            return 1;
        }
        h = h.wrapping_add(1);
    }
}

/// JIT GROUP BY engine: compiles tight loop + extern-C accumulate call.
pub struct GroupByJitEngine {
    module: JITModule,
    func_id: Option<FuncId>,
}

impl GroupByJitEngine {
    pub fn new() -> Result<Self> {
        let module = make_module()?;
        Ok(Self { module, func_id: None })
    }

    /// Compile GROUP BY SUM plan. Signature:
    ///   fn(rows_ptr: *const i64, ncols: i64, nrows: i64,
    ///      bucket_ptr: *mut i64, n_buckets: i64) -> ()
    pub fn compile(&mut self, plan: &GroupBySumPlan) -> Result<FuncId> {
        if let Some(id) = self.func_id {
            return Ok(id);
        }

        let ptr_type = self.module.target_config().pointer_type();

        // Declare extern jit_gb_accumulate
        let mut acc_sig = self.module.make_signature();
        acc_sig.params.push(AbiParam::new(ptr_type));   // bucket_ptr
        acc_sig.params.push(AbiParam::new(types::I64)); // n_buckets
        acc_sig.params.push(AbiParam::new(types::I64)); // key
        acc_sig.params.push(AbiParam::new(types::I64)); // val
        let acc_id = self.module.declare_function(
            "jit_gb_accumulate", Linkage::Import, &acc_sig,
        ).context("declare jit_gb_accumulate")?;

        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(ptr_type));   // rows_ptr
        sig.params.push(AbiParam::new(types::I64)); // ncols
        sig.params.push(AbiParam::new(types::I64)); // nrows
        sig.params.push(AbiParam::new(ptr_type));   // bucket_ptr
        sig.params.push(AbiParam::new(types::I64)); // n_buckets

        let col_group = plan.col_group as i64;
        let col_agg   = plan.col_agg   as i64;

        let func_id = self.module.declare_function(
            "gb_sum_loop", Linkage::Local, &sig,
        ).context("declare gb_sum_loop")?;

        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;

        {
            let mut fbc = FunctionBuilderContext::new();
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut fbc);

            let entry  = b.create_block();
            let header = b.create_block();
            let body   = b.create_block();
            let end    = b.create_block();

            b.append_block_params_for_function_params(entry);
            b.switch_to_block(entry);
            b.seal_block(entry);

            let ps = b.block_params(entry).to_vec();
            let rows_ptr   = ps[0];
            let ncols      = ps[1];
            let nrows      = ps[2];
            let bucket_ptr = ps[3];
            let n_buckets  = ps[4];

            let i_slot = b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3));
            let zero = b.ins().iconst(types::I64, 0);
            b.ins().stack_store(zero, i_slot, 0);
            b.ins().jump(header, &[]);

            // header: i < nrows ?
            b.switch_to_block(header);
            let i = b.ins().stack_load(types::I64, i_slot, 0);
            let cond = b.ins().icmp(IntCC::SignedLessThan, i, nrows);
            b.ins().brif(cond, body, &[], end, &[]);

            // body: extract key + val, call jit_gb_accumulate
            b.switch_to_block(body);
            let ncols_x8  = b.ins().imul_imm(ncols, 8);
            let row_off   = b.ins().imul(i, ncols_x8);
            let row_base  = b.ins().iadd(rows_ptr, row_off);

            let key_off = b.ins().iconst(types::I64, col_group * 8);
            let key_addr = b.ins().iadd(row_base, key_off);
            let key = b.ins().load(types::I64, MemFlags::trusted(), key_addr, 0);

            let val_off = b.ins().iconst(types::I64, col_agg * 8);
            let val_addr = b.ins().iadd(row_base, val_off);
            let val = b.ins().load(types::I64, MemFlags::trusted(), val_addr, 0);

            let acc_ref = self.module.declare_func_in_func(acc_id, b.func);
            b.ins().call(acc_ref, &[bucket_ptr, n_buckets, key, val]);

            let one = b.ins().iconst(types::I64, 1);
            let i_cur = b.ins().stack_load(types::I64, i_slot, 0);
            let i_next = b.ins().iadd(i_cur, one);
            b.ins().stack_store(i_next, i_slot, 0);
            b.ins().jump(header, &[]);

            b.switch_to_block(end);
            b.ins().return_(&[]);

            b.seal_all_blocks();
            b.finalize();
        }

        self.module.define_function(func_id, &mut ctx).context("define gb_sum")?;
        self.module.clear_context(&mut ctx);
        self.module.finalize_definitions().context("finalize gb")?;
        self.func_id = Some(func_id);
        Ok(func_id)
    }

    /// Execute GROUP BY SUM. Returns (key, sum) sorted by key.
    pub fn execute(&self, func_id: FuncId, plan: &GroupBySumPlan, rows: &[Row]) -> Result<Vec<(i64, i64)>> {
        if rows.is_empty() { return Ok(vec![]); }
        let ncols = rows[0].0.len();
        let mut flat: Vec<i64> = Vec::with_capacity(ncols * rows.len());
        for r in rows { flat.extend_from_slice(&r.0); }

        // Estimate cardinality: next power of 2, min 16
        let card = rows.len().next_power_of_two().max(16);
        let n_buckets = card * 2; // load factor ~0.5
        // flat open-address: [key, sum] pairs; i64::MIN = empty
        let mut buckets: Vec<i64> = vec![i64::MIN; n_buckets * 2];

        let fn_ptr = self.module.get_finalized_function(func_id);
        let compiled: unsafe extern "C" fn(*const i64, i64, i64, *mut i64, i64) =
            unsafe { std::mem::transmute(fn_ptr) };

        unsafe {
            compiled(
                flat.as_ptr(),
                ncols as i64,
                rows.len() as i64,
                buckets.as_mut_ptr(),
                n_buckets as i64,
            );
        }

        let mut out: Vec<(i64, i64)> = buckets.chunks_exact(2)
            .filter(|p| p[0] != i64::MIN)
            .map(|p| (p[0], p[1]))
            .collect();
        out.sort_unstable_by_key(|p| p.0);
        Ok(out)
    }
}

/// JIT HASH JOIN engine
pub struct HashJoinJitEngine {
    module: JITModule,
    func_id: Option<FuncId>,
}

impl HashJoinJitEngine {
    pub fn new() -> Result<Self> {
        let module = make_module()?;
        Ok(Self { module, func_id: None })
    }

    /// Compile hash join probe loop. Signature:
    ///   fn(rows_ptr: *const i64, ncols: i64, nrows: i64,
    ///      set_ptr: *const i64, n_buckets: i64,
    ///      out_ptr: *mut i64, out_len_ptr: *mut i64) -> ()
    pub fn compile(&mut self, plan: &HashJoinPlan) -> Result<FuncId> {
        if let Some(id) = self.func_id { return Ok(id); }

        let ptr_type = self.module.target_config().pointer_type();

        // Declare extern jit_hj_probe
        let mut probe_sig = self.module.make_signature();
        probe_sig.params.push(AbiParam::new(ptr_type));   // set_ptr
        probe_sig.params.push(AbiParam::new(types::I64)); // n_buckets
        probe_sig.params.push(AbiParam::new(types::I64)); // key
        probe_sig.returns.push(AbiParam::new(types::I64)); // found
        let probe_id = self.module.declare_function(
            "jit_hj_probe", Linkage::Import, &probe_sig,
        ).context("declare jit_hj_probe")?;

        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(ptr_type));   // rows_ptr
        sig.params.push(AbiParam::new(types::I64)); // ncols
        sig.params.push(AbiParam::new(types::I64)); // nrows
        sig.params.push(AbiParam::new(ptr_type));   // set_ptr
        sig.params.push(AbiParam::new(types::I64)); // n_buckets
        sig.params.push(AbiParam::new(ptr_type));   // out_ptr
        sig.params.push(AbiParam::new(ptr_type));   // out_len_ptr

        let left_key = plan.left_key as i64;

        let func_id = self.module.declare_function(
            "hj_probe_loop", Linkage::Local, &sig,
        ).context("declare hj_probe_loop")?;

        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;

        {
            let mut fbc = FunctionBuilderContext::new();
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut fbc);

            let entry   = b.create_block();
            let header  = b.create_block();
            let body    = b.create_block();
            let matched = b.create_block();
            let lend    = b.create_block();
            let exit    = b.create_block();

            b.append_block_params_for_function_params(entry);
            b.switch_to_block(entry);
            b.seal_block(entry);

            let ps = b.block_params(entry).to_vec();
            let rows_ptr   = ps[0];
            let ncols      = ps[1];
            let nrows      = ps[2];
            let set_ptr    = ps[3];
            let n_buckets  = ps[4];
            let out_ptr    = ps[5];
            let out_len_p  = ps[6];

            let i_slot  = b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3));
            let oc_slot = b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3));
            let zero = b.ins().iconst(types::I64, 0);
            b.ins().stack_store(zero, i_slot, 0);
            b.ins().stack_store(zero, oc_slot, 0);
            b.ins().jump(header, &[]);

            b.switch_to_block(header);
            let i = b.ins().stack_load(types::I64, i_slot, 0);
            let cond = b.ins().icmp(IntCC::SignedLessThan, i, nrows);
            b.ins().brif(cond, body, &[], exit, &[]);

            b.switch_to_block(body);
            let ncols_x8 = b.ins().imul_imm(ncols, 8);
            let row_off  = b.ins().imul(i, ncols_x8);
            let row_base = b.ins().iadd(rows_ptr, row_off);

            let key_off  = b.ins().iconst(types::I64, left_key * 8);
            let key_addr = b.ins().iadd(row_base, key_off);
            let key = b.ins().load(types::I64, MemFlags::trusted(), key_addr, 0);

            let probe_ref = self.module.declare_func_in_func(probe_id, b.func);
            let call = b.ins().call(probe_ref, &[set_ptr, n_buckets, key]);
            let found = b.inst_results(call)[0];
            let found_bool = b.ins().icmp_imm(IntCC::NotEqual, found, 0);
            b.ins().brif(found_bool, matched, &[], lend, &[]);

            // matched: copy full row to out
            b.switch_to_block(matched);
            let oc = b.ins().stack_load(types::I64, oc_slot, 0);
            // ncols known at exec time, loop unroll not needed — use runtime ncols
            // We copy row by calling memcpy-like byte-by-byte via Cranelift
            // Simple: emit store for each col via known ncols at compile time
            // But ncols is runtime value here — so we use a simpler approach:
            // store key col only (simplification: full row copy via byte-level loop)
            // For bench accuracy we copy ncols cols using dynamic loop
            // (This is still JIT — cranelift emits the copy loop natively)
            let copy_slot = b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3));
            b.ins().stack_store(zero, copy_slot, 0);
            let copy_hdr = b.create_block();
            let copy_body = b.create_block();
            let copy_end = b.create_block();
            b.ins().jump(copy_hdr, &[]);

            b.switch_to_block(copy_hdr);
            let j = b.ins().stack_load(types::I64, copy_slot, 0);
            let jcond = b.ins().icmp(IntCC::SignedLessThan, j, ncols);
            b.ins().brif(jcond, copy_body, &[], copy_end, &[]);

            b.switch_to_block(copy_body);
            let src_off  = b.ins().imul_imm(j, 8);
            let src_addr = b.ins().iadd(row_base, src_off);
            let col_val  = b.ins().load(types::I64, MemFlags::trusted(), src_addr, 0);
            let dst_off  = b.ins().imul(oc, ncols);
            let dst_idx  = b.ins().iadd(dst_off, j);
            let dst_byte = b.ins().imul_imm(dst_idx, 8);
            let dst_addr = b.ins().iadd(out_ptr, dst_byte);
            b.ins().store(MemFlags::trusted(), col_val, dst_addr, 0);
            let one = b.ins().iconst(types::I64, 1);
            let j_next = b.ins().iadd(j, one);
            b.ins().stack_store(j_next, copy_slot, 0);
            b.ins().jump(copy_hdr, &[]);

            b.switch_to_block(copy_end);
            let one = b.ins().iconst(types::I64, 1);
            let oc_new = b.ins().iadd(oc, one);
            b.ins().stack_store(oc_new, oc_slot, 0);
            b.ins().jump(lend, &[]);

            b.switch_to_block(lend);
            let i_cur  = b.ins().stack_load(types::I64, i_slot, 0);
            let i_next = b.ins().iadd_imm(i_cur, 1);
            b.ins().stack_store(i_next, i_slot, 0);
            b.ins().jump(header, &[]);

            b.switch_to_block(exit);
            let final_oc = b.ins().stack_load(types::I64, oc_slot, 0);
            b.ins().store(MemFlags::trusted(), final_oc, out_len_p, 0);
            b.ins().return_(&[]);

            b.seal_all_blocks();
            b.finalize();
        }

        self.module.define_function(func_id, &mut ctx).context("define hj")?;
        self.module.clear_context(&mut ctx);
        self.module.finalize_definitions().context("finalize hj")?;
        self.func_id = Some(func_id);
        Ok(func_id)
    }

    /// Build hash set from right side, then probe with JIT loop.
    pub fn execute(
        &self,
        func_id: FuncId,
        plan: &HashJoinPlan,
        left: &[Row],
        right: &[Row],
    ) -> Result<Vec<Row>> {
        if left.is_empty() || right.is_empty() { return Ok(vec![]); }
        let ncols = left[0].0.len();

        // Build phase (Rust): open-address set of right keys
        let set_size = right.len().next_power_of_two().max(16) * 2;
        let mut set: Vec<i64> = vec![i64::MIN; set_size];
        let mask = set_size - 1;
        for r in right {
            let key = r.0[plan.right_key];
            let mut h = (key as usize).wrapping_mul(0x9e3779b97f4a7c15);
            loop {
                let slot = h & mask;
                if set[slot] == i64::MIN { set[slot] = key; break; }
                if set[slot] == key { break; }
                h = h.wrapping_add(1);
            }
        }

        // Flatten left
        let mut flat: Vec<i64> = Vec::with_capacity(ncols * left.len());
        for r in left { flat.extend_from_slice(&r.0); }

        let mut out_buf: Vec<i64> = vec![0i64; ncols * left.len()];
        let mut out_len: i64 = 0;

        let fn_ptr = self.module.get_finalized_function(func_id);
        let compiled: unsafe extern "C" fn(*const i64, i64, i64, *const i64, i64, *mut i64, *mut i64) =
            unsafe { std::mem::transmute(fn_ptr) };

        unsafe {
            compiled(
                flat.as_ptr(),
                ncols as i64,
                left.len() as i64,
                set.as_ptr(),
                set_size as i64,
                out_buf.as_mut_ptr(),
                &mut out_len as *mut i64,
            );
        }

        let n = out_len as usize;
        Ok((0..n).map(|r| Row(out_buf[r * ncols..(r + 1) * ncols].to_vec())).collect())
    }
}

fn make_module() -> Result<JITModule> {
    let mut flag_builder = settings::builder();
    flag_builder.set("use_colocated_libcalls", "false").unwrap();
    flag_builder.set("is_pic", "false").unwrap();
    flag_builder.set("opt_level", "speed").unwrap();
    let isa_flags = settings::Flags::new(flag_builder);
    let isa = cranelift_native::builder()
        .map_err(|e| anyhow::anyhow!("ISA: {e}"))?
        .finish(isa_flags)
        .map_err(|e| anyhow::anyhow!("ISA finish: {e}"))?;
    let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
    // Register extern symbols
    builder.symbol("jit_gb_accumulate", jit_gb_accumulate as *const u8);
    builder.symbol("jit_hj_probe", jit_hj_probe as *const u8);
    Ok(JITModule::new(builder))
}

pub struct JitEngine {
    module: JITModule,
    cache: HashMap<u64, FuncId>,
}

impl JitEngine {
    pub fn new() -> Result<Self> {
        let module = make_module()?;
        Ok(JitEngine { module, cache: HashMap::new() })
    }

    /// Compile a QueryPlan for a given schema. Returns FuncId (cached by blake3).
    pub fn compile(&mut self, plan: &QueryPlan, schema: &Schema) -> Result<FuncId> {
        let key = plan.fingerprint(schema.columns.len());
        if let Some(&id) = self.cache.get(&key) {
            return Ok(id);
        }

        let ptr_type = self.module.target_config().pointer_type();
        let mut sig = self.module.make_signature();
        // rows_ptr, ncols, nrows, out_ptr, out_len_ptr
        sig.params.push(AbiParam::new(ptr_type));  // rows_ptr
        sig.params.push(AbiParam::new(types::I64)); // ncols
        sig.params.push(AbiParam::new(types::I64)); // nrows
        sig.params.push(AbiParam::new(ptr_type));  // out_ptr
        sig.params.push(AbiParam::new(ptr_type));  // out_len_ptr

        let func_name = format!("query_{key:016x}");
        let func_id = self.module
            .declare_function(&func_name, Linkage::Local, &sig)
            .context("declare_function")?;

        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;

        {
            let mut fn_builder_ctx = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_builder_ctx);

            // Blocks
            let entry_block  = builder.create_block();
            let loop_header  = builder.create_block();
            let loop_body    = builder.create_block();
            let filter_pass  = builder.create_block();
            let loop_end     = builder.create_block();
            let exit_block   = builder.create_block();

            // Entry
            builder.append_block_params_for_function_params(entry_block);
            builder.switch_to_block(entry_block);
            builder.seal_block(entry_block);

            let params = builder.block_params(entry_block).to_vec();
            let rows_ptr   = params[0];
            let ncols      = params[1];
            let nrows      = params[2];
            let out_ptr    = params[3];
            let out_len    = params[4];

            // i = 0  (row index)
            let i_slot = builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3));
            let zero64 = builder.ins().iconst(types::I64, 0);
            builder.ins().stack_store(zero64, i_slot, 0);

            // out_count = 0
            let oc_slot = builder.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3));
            builder.ins().stack_store(zero64, oc_slot, 0);

            builder.ins().jump(loop_header, &[]);

            // ── loop_header: check i < nrows ──────────────────────────────
            builder.switch_to_block(loop_header);
            let i_val = builder.ins().stack_load(types::I64, i_slot, 0);
            let cond  = builder.ins().icmp(IntCC::SignedLessThan, i_val, nrows);
            builder.ins().brif(cond, loop_body, &[], exit_block, &[]);

            // ── loop_body: load row columns ───────────────────────────────
            builder.switch_to_block(loop_body);

            // row_base_ptr = rows_ptr + i * ncols * 8
            let ncols_x8   = builder.ins().imul_imm(ncols, 8);
            let row_offset = builder.ins().imul(i_val, ncols_x8);
            let row_base   = builder.ins().iadd(rows_ptr, row_offset);

            let ncols_usize = schema.columns.len();

            // Helper: load column j from row
            let load_col = |b: &mut FunctionBuilder, col: usize| -> Value_ {
                let off = (col as i64) * 8;
                let offset = b.ins().iconst(types::I64, off);
                let addr   = b.ins().iadd(row_base, offset);
                b.ins().load(types::I64, MemFlags::trusted(), addr, 0)
            };

            // Evaluate filter predicate
            let keep = if let Some(pred) = &plan.filter {
                emit_expr(&mut builder, pred, row_base, ncols_usize)
            } else {
                builder.ins().iconst(types::I64, 1)
            };

            let keep_bool = builder.ins().icmp_imm(IntCC::NotEqual, keep, 0);
            builder.ins().brif(keep_bool, filter_pass, &[], loop_end, &[]);

            // ── filter_pass: write row to output ──────────────────────────
            builder.switch_to_block(filter_pass);

            let out_count = builder.ins().stack_load(types::I64, oc_slot, 0);

            if plan.projections.is_empty() {
                // SELECT * — copy all columns
                for j in 0..ncols_usize {
                    let val = load_col(&mut builder, j);
                    // out_ptr[(out_count * ncols + j) * 8]
                    let row_out_offset = builder.ins().imul(out_count, ncols);
                    let col_out_idx    = builder.ins().iadd_imm(row_out_offset, j as i64);
                    let byte_off       = builder.ins().imul_imm(col_out_idx, 8);
                    let out_addr       = builder.ins().iadd(out_ptr, byte_off);
                    builder.ins().store(MemFlags::trusted(), val, out_addr, 0);
                }
            } else {
                // Projected columns — out has proj.len() cols per row
                let proj_len = plan.projections.len() as i64;
                for (j, proj) in plan.projections.iter().enumerate() {
                    let val = emit_expr(&mut builder, proj, row_base, ncols_usize);
                    let row_out_offset = builder.ins().imul_imm(out_count, proj_len);
                    let col_out_idx    = builder.ins().iadd_imm(row_out_offset, j as i64);
                    let byte_off       = builder.ins().imul_imm(col_out_idx, 8);
                    let out_addr       = builder.ins().iadd(out_ptr, byte_off);
                    builder.ins().store(MemFlags::trusted(), val, out_addr, 0);
                }
            }

            let one = builder.ins().iconst(types::I64, 1);
            let new_oc = builder.ins().iadd(out_count, one);
            builder.ins().stack_store(new_oc, oc_slot, 0);
            builder.ins().jump(loop_end, &[]);

            // ── loop_end: i++ ─────────────────────────────────────────────
            builder.switch_to_block(loop_end);
            let i_cur  = builder.ins().stack_load(types::I64, i_slot, 0);
            let i_next = builder.ins().iadd_imm(i_cur, 1);
            builder.ins().stack_store(i_next, i_slot, 0);
            builder.ins().jump(loop_header, &[]);

            // ── exit: store out_count ─────────────────────────────────────
            builder.switch_to_block(exit_block);
            let final_oc = builder.ins().stack_load(types::I64, oc_slot, 0);
            builder.ins().store(MemFlags::trusted(), final_oc, out_len, 0);
            builder.ins().return_(&[]);

            builder.seal_all_blocks();
            builder.finalize();
        }

        self.module
            .define_function(func_id, &mut ctx)
            .context("define_function")?;
        self.module.clear_context(&mut ctx);
        self.module.finalize_definitions().context("finalize_definitions")?;

        self.cache.insert(key, func_id);
        Ok(func_id)
    }

    /// Execute a compiled function against input rows.
    pub fn execute(&self, func_id: FuncId, plan: &QueryPlan, rows: &[Row]) -> Result<Vec<Row>> {
        if rows.is_empty() {
            return Ok(vec![]);
        }
        let ncols = rows[0].0.len();
        let nrows = rows.len();

        // Flatten rows → contiguous i64 slice
        let mut flat: Vec<i64> = Vec::with_capacity(ncols * nrows);
        for r in rows {
            flat.extend_from_slice(&r.0);
        }

        let out_cols = if plan.projections.is_empty() { ncols } else { plan.projections.len() };
        let mut out_buf: Vec<i64> = vec![0i64; out_cols * nrows];
        let mut out_len: i64 = 0;

        let fn_ptr = self.module.get_finalized_function(func_id);

        // SAFETY: we compiled this function; ABI matches signature above.
        let compiled_fn: unsafe extern "C" fn(*const i64, i64, i64, *mut i64, *mut i64) =
            unsafe { std::mem::transmute(fn_ptr) };

        unsafe {
            compiled_fn(
                flat.as_ptr(),
                ncols as i64,
                nrows as i64,
                out_buf.as_mut_ptr(),
                &mut out_len as *mut i64,
            );
        }

        let n = out_len as usize;
        let result = (0..n)
            .map(|r| Row(out_buf[r * out_cols..(r + 1) * out_cols].to_vec()))
            .collect();
        Ok(result)
    }
}

// ── IR → Cranelift codegen helper ──────────────────────────────────────────

type Value_ = cranelift::prelude::Value;

fn emit_expr(
    b: &mut FunctionBuilder,
    expr: &Expr,
    row_base: Value_,
    _ncols: usize,
) -> Value_ {
    match expr {
        Expr::Col(i) => {
            let off = (*i as i64) * 8;
            let offset = b.ins().iconst(types::I64, off);
            let addr   = b.ins().iadd(row_base, offset);
            b.ins().load(types::I64, MemFlags::trusted(), addr, 0)
        }
        Expr::Const(v) => b.ins().iconst(types::I64, *v),
        Expr::Mul(inner, factor) => {
            let v = emit_expr(b, inner, row_base, _ncols);
            b.ins().imul_imm(v, *factor)
        }
        Expr::Cmp(lhs, op, rhs) => {
            let l = emit_expr(b, lhs, row_base, _ncols);
            let r = emit_expr(b, rhs, row_base, _ncols);
            let cc = match op {
                CmpOp::Gt  => IntCC::SignedGreaterThan,
                CmpOp::Gte => IntCC::SignedGreaterThanOrEqual,
                CmpOp::Lt  => IntCC::SignedLessThan,
                CmpOp::Lte => IntCC::SignedLessThanOrEqual,
                CmpOp::Eq  => IntCC::Equal,
                CmpOp::Neq => IntCC::NotEqual,
            };
            let cond = b.ins().icmp(cc, l, r);
            b.ins().uextend(types::I64, cond)
        }
    }
}
