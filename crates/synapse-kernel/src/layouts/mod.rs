//! Layouts: AoS (vec-major), SoA (dim-major), blocked (cache-line tiled).

use std::alloc::{Layout, alloc, dealloc};

/// 64-byte (cache-line) aligned heap buffer of f32.
/// Required for peak NEON / AVX2 throughput via aligned loads.
pub struct AlignedF32 {
    ptr: *mut f32,
    len: usize,
    cap: usize,
}

// SAFETY: single-owner, no shared state.
unsafe impl Send for AlignedF32 {}
unsafe impl Sync for AlignedF32 {}

impl AlignedF32 {
    pub fn new(len: usize) -> Self {
        if len == 0 {
            return Self {
                ptr: std::ptr::NonNull::dangling().as_ptr(),
                len: 0,
                cap: 0,
            };
        }
        let layout = Layout::from_size_align(len * 4, 64).expect("layout");
        // SAFETY: layout is non-zero, aligned to 64.
        let ptr = unsafe { alloc(layout) as *mut f32 };
        assert!(!ptr.is_null(), "allocation failed");
        Self { ptr, len, cap: len }
    }

    #[inline(always)]
    pub fn as_slice(&self) -> &[f32] {
        // SAFETY: ptr valid for `len` f32 values.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    #[inline(always)]
    pub fn as_mut_slice(&mut self) -> &mut [f32] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Drop for AlignedF32 {
    fn drop(&mut self) {
        if self.cap > 0 {
            let layout = Layout::from_size_align(self.cap * 4, 64).expect("layout");
            unsafe { dealloc(self.ptr as *mut u8, layout) };
        }
    }
}

impl From<&[f32]> for AlignedF32 {
    fn from(src: &[f32]) -> Self {
        let mut a = AlignedF32::new(src.len());
        a.as_mut_slice().copy_from_slice(src);
        a
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_64() {
        let a = AlignedF32::new(128);
        assert_eq!(a.ptr as usize % 64, 0);
    }

    #[test]
    fn roundtrip() {
        let src = vec![1.0f32, 2.0, 3.0, 4.0];
        let a = AlignedF32::from(src.as_slice());
        assert_eq!(a.as_slice(), &[1.0, 2.0, 3.0, 4.0]);
    }
}
