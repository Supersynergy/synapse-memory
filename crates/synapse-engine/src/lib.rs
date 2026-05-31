pub mod abi;
pub mod rrf;

pub use abi::{synapse_engine_init, synapse_engine_score, synapse_engine_version};

/// Safe Rust wrapper around the C-ABI RRF fuse — useful for intra-process callers.
pub fn rrf_fuse_safe(a: &[f64], b: &[f64], k: f64) -> Vec<f64> {
    let cap = a.len().max(b.len());
    let mut out = vec![0.0_f64; cap];
    if cap == 0 {
        return out;
    }
    let n = unsafe {
        abi::synapse_engine_rrf_fuse(
            a.as_ptr(),
            a.len(),
            b.as_ptr(),
            b.len(),
            k,
            out.as_mut_ptr(),
            cap,
        )
    };
    out.truncate(n.max(0) as usize);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrf_fuse_safe_roundtrip() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0];
        let out = rrf_fuse_safe(&a, &b, 60.0);
        assert_eq!(out.len(), 3);
        // index 0: both lists contribute
        let expected0 = 1.0 / 61.0 + 1.0 / 61.0;
        assert!((out[0] - expected0).abs() < 1e-12);
        // index 2: only a contributes
        assert!((out[2] - 1.0 / 63.0).abs() < 1e-12);
    }

    #[test]
    fn rrf_fuse_safe_empty() {
        let out = rrf_fuse_safe(&[], &[], 60.0);
        assert!(out.is_empty());
    }
}
