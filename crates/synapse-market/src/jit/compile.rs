use cranelift_codegen::ir::{
    types, AbiParam, BlockArg, Function, InstBuilder, MemFlags, UserFuncName, Value,
};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};

use crate::jit::predicate::{Col, Op, Predicate};

pub type CompiledFilter = unsafe extern "C" fn(
    ts_ptr: *const i64,
    open_ptr: *const f32,
    high_ptr: *const f32,
    low_ptr: *const f32,
    close_ptr: *const f32,
    volume_ptr: *const f32,
    n: usize,
    out_mask: *mut u8,
) -> usize;

/// Holds JIT module alive alongside the compiled fn ptr.
pub struct CompiledFn {
    _module: Box<JITModule>,
    pub func_ptr: CompiledFilter,
}

pub fn compile(p: &Predicate) -> anyhow::Result<CompiledFn> {
    let mut flag_builder = cranelift_codegen::settings::builder();
    flag_builder.set("use_colocated_libcalls", "false").unwrap();
    flag_builder.set("is_pic", "false").unwrap();
    let flags = cranelift_codegen::settings::Flags::new(flag_builder);
    let isa = cranelift_jit::host_isa_builder()
        .map_err(|e| anyhow::anyhow!("ISA: {e}"))?
        .finish(flags)
        .map_err(|e| anyhow::anyhow!("ISA finish: {e}"))?;

    let jit_builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
    let mut module = Box::new(JITModule::new(jit_builder));

    let ptr = module.isa().pointer_type();
    let mut sig = module.make_signature();
    for _ in 0..8 {
        sig.params.push(AbiParam::new(ptr));
    }
    sig.returns.push(AbiParam::new(ptr));

    let func_id = module
        .declare_function("filter", Linkage::Export, &sig)
        .map_err(|e| anyhow::anyhow!("declare: {e}"))?;

    let mut func = Function::with_name_signature(UserFuncName::user(0, func_id.as_u32()), sig);
    let mut fb_ctx = FunctionBuilderContext::new();
    {
        let mut b = FunctionBuilder::new(&mut func, &mut fb_ctx);

        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        b.seal_block(entry);

        let params: Vec<Value> = b.block_params(entry).to_vec();
        let ts_ptr    = params[0];
        let open_ptr  = params[1];
        let high_ptr  = params[2];
        let low_ptr   = params[3];
        let close_ptr = params[4];
        let vol_ptr   = params[5];
        let n         = params[6];
        let out_ptr   = params[7];

        let loop_hdr  = b.create_block();
        let loop_body = b.create_block();
        let loop_exit = b.create_block();

        // loop_hdr(i: ptr, count: ptr)
        b.append_block_param(loop_hdr, ptr);
        b.append_block_param(loop_hdr, ptr);
        // loop_exit(count: ptr)
        b.append_block_param(loop_exit, ptr);

        let zero = b.ins().iconst(ptr, 0);
        b.ins().jump(loop_hdr, &[BlockArg::Value(zero), BlockArg::Value(zero)]);

        // -- header --
        b.switch_to_block(loop_hdr);
        let i     = b.block_params(loop_hdr)[0];
        let count = b.block_params(loop_hdr)[1];
        let lt = b.ins().icmp(
            cranelift_codegen::ir::condcodes::IntCC::UnsignedLessThan,
            i, n,
        );
        b.ins().brif(lt, loop_body, &[], loop_exit, &[count]);
        b.seal_block(loop_hdr);

        // -- body --
        b.switch_to_block(loop_body);
        b.seal_block(loop_body);
        // re-fetch params from loop_hdr
        let i_v     = b.block_params(loop_hdr)[0];
        let count_v = b.block_params(loop_hdr)[1];

        let match_val = emit_predicate(
            &mut b, p, i_v,
            ts_ptr, open_ptr, high_ptr, low_ptr, close_ptr, vol_ptr,
            ptr,
        );
        let byte_val = b.ins().ireduce(types::I8, match_val);
        let addr = b.ins().iadd(out_ptr, i_v);
        b.ins().store(MemFlags::trusted(), byte_val, addr, 0i32);

        let match_ext = b.ins().uextend(ptr, byte_val);
        let new_count = b.ins().iadd(count_v, match_ext);
        let one = b.ins().iconst(ptr, 1);
        let next_i = b.ins().iadd(i_v, one);
        b.ins().jump(loop_hdr, &[next_i, new_count]);

        // -- exit --
        b.switch_to_block(loop_exit);
        b.seal_block(loop_exit);
        let final_count = b.block_params(loop_exit)[0];
        b.ins().return_(&[final_count]);

        b.finalize();
    }

    let mut ctx = Context::for_function(func);
    module
        .define_function(func_id, &mut ctx)
        .map_err(|e| anyhow::anyhow!("define: {e}"))?;
    module.finalize_definitions().map_err(|e| anyhow::anyhow!("finalize: {e}"))?;

    let raw = module.get_finalized_function(func_id);
    let func_ptr: CompiledFilter = unsafe { std::mem::transmute(raw) };

    Ok(CompiledFn { _module: module, func_ptr })
}

fn emit_predicate(
    b: &mut FunctionBuilder,
    p: &Predicate,
    i: Value,
    ts_ptr: Value,
    open_ptr: Value,
    high_ptr: Value,
    low_ptr: Value,
    close_ptr: Value,
    vol_ptr: Value,
    ptr: types::Type,
) -> Value {
    match p {
        Predicate::Cmp(col, op, val) => {
            let (base, is_ts) = match col {
                Col::Ts     => (ts_ptr, true),
                Col::Open   => (open_ptr, false),
                Col::High   => (high_ptr, false),
                Col::Low    => (low_ptr, false),
                Col::Close  => (close_ptr, false),
                Col::Volume => (vol_ptr, false),
            };
            let cond = if is_ts {
                let stride = b.ins().iconst(ptr, 8);
                let off = b.ins().imul(i, stride);
                let addr = b.ins().iadd(base, off);
                let loaded = b.ins().load(types::I64, MemFlags::trusted(), addr, 0i32);
                let rhs = b.ins().iconst(types::I64, *val as i64);
                let icc = int_cc(op);
                b.ins().icmp(icc, loaded, rhs)
            } else {
                let stride = b.ins().iconst(ptr, 4);
                let off = b.ins().imul(i, stride);
                let addr = b.ins().iadd(base, off);
                let loaded = b.ins().load(types::F32, MemFlags::trusted(), addr, 0i32);
                let rhs = b.ins().f32const(*val);
                let fcc = float_cc(op);
                b.ins().fcmp(fcc, loaded, rhs)
            };
            b.ins().uextend(ptr, cond)
        }
        Predicate::And(a, bx) => {
            let va = emit_predicate(b, a, i, ts_ptr, open_ptr, high_ptr, low_ptr, close_ptr, vol_ptr, ptr);
            let vb = emit_predicate(b, bx, i, ts_ptr, open_ptr, high_ptr, low_ptr, close_ptr, vol_ptr, ptr);
            b.ins().band(va, vb)
        }
        Predicate::Or(a, bx) => {
            let va = emit_predicate(b, a, i, ts_ptr, open_ptr, high_ptr, low_ptr, close_ptr, vol_ptr, ptr);
            let vb = emit_predicate(b, bx, i, ts_ptr, open_ptr, high_ptr, low_ptr, close_ptr, vol_ptr, ptr);
            b.ins().bor(va, vb)
        }
        Predicate::Not(inner) => {
            let v = emit_predicate(b, inner, i, ts_ptr, open_ptr, high_ptr, low_ptr, close_ptr, vol_ptr, ptr);
            let one = b.ins().iconst(ptr, 1);
            b.ins().bxor(v, one)
        }
    }
}

fn int_cc(op: &Op) -> cranelift_codegen::ir::condcodes::IntCC {
    use cranelift_codegen::ir::condcodes::IntCC;
    match op {
        Op::Lt => IntCC::SignedLessThan,
        Op::Le => IntCC::SignedLessThanOrEqual,
        Op::Eq => IntCC::Equal,
        Op::Ge => IntCC::SignedGreaterThanOrEqual,
        Op::Gt => IntCC::SignedGreaterThan,
        Op::Ne => IntCC::NotEqual,
    }
}

fn float_cc(op: &Op) -> cranelift_codegen::ir::condcodes::FloatCC {
    use cranelift_codegen::ir::condcodes::FloatCC;
    match op {
        Op::Lt => FloatCC::LessThan,
        Op::Le => FloatCC::LessThanOrEqual,
        Op::Eq => FloatCC::Equal,
        Op::Ge => FloatCC::GreaterThanOrEqual,
        Op::Gt => FloatCC::GreaterThan,
        Op::Ne => FloatCC::NotEqual,
    }
}
