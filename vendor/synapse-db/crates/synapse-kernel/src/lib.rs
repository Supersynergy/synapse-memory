//! synapse-kernel — ns-tier hot-path distance kernels.
//!
//! Goal: per-vec dist <2 ns @ L1 (memcpy-parity).
//! Stages: branchless → prefetch → cache-align → unroll-FMA → NEON FMLAL2 → AMX → cascade.
//!
//! See `bench/ns_microbench/README.md` for targets & harness plan.

pub mod kernels;
pub mod layouts;
pub mod workloads;

/// Public re-export: 64-byte aligned f32 buffer.
pub use layouts::AlignedF32;

/// Public re-export: NEON-dispatched i8 dot product.
pub use kernels::i8_dot::dot_i8;
pub use kernels::i8_dot::dot_i8_scalar;

/// Public re-export: NEON-dispatched f16 dot product.
pub use kernels::f16_dot::dot_f16;
pub use kernels::f16_dot::dot_f16_scalar;

#[inline(always)]
pub fn prefetch<T>(p: *const T) {
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!("prfm pldl1keep, [{0}]", in(reg) p, options(nostack, preserves_flags));
    }
    #[cfg(not(target_arch = "aarch64"))]
    let _ = p;
}
