pub mod binary;
pub mod cache;
pub mod embed;
pub mod embed_mlx;
pub mod error;
pub mod http;
pub mod index;
pub mod rabitq;
pub mod search;
pub mod snapshot;
pub mod socket;
pub mod vec_socket;

pub use error::UltraError;
pub use index::UltraIndex;

// ── C-ABI FFI exports ────────────────────────────────────────────────────────

use arc_swap::ArcSwap;
use std::sync::Arc;

/// Opaque handle returned by `synapse_open`.
pub struct SynapseHandle {
    index: Arc<ArcSwap<UltraIndex>>,
}

/// Load index from snapshot file. Returns null on failure.
/// Caller must free with `synapse_close`.
///
/// # Safety
/// `snap_path` must be a valid, NUL-terminated C string pointer or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn synapse_open(
    snap_path: *const std::os::raw::c_char,
) -> *mut SynapseHandle {
    let path_str = unsafe {
        if snap_path.is_null() {
            return std::ptr::null_mut();
        }
        std::ffi::CStr::from_ptr(snap_path).to_str().unwrap_or("")
    };
    let snap_path = std::path::Path::new(path_str);
    let brain_mtime = 0u64; // always load snapshot, skip freshness check
    let snap = match snapshot::load_mmap(snap_path, brain_mtime) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    let idx = UltraIndex::from_snapshot(snap);
    let handle = Box::new(SynapseHandle {
        index: Arc::new(ArcSwap::from(Arc::new(idx))),
    });
    Box::into_raw(handle)
}

/// Free handle.
///
/// # Safety
/// `handle` must be null or a pointer previously returned by `synapse_open` and not freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn synapse_close(handle: *mut SynapseHandle) {
    if !handle.is_null() {
        unsafe {
            drop(Box::from_raw(handle));
        }
    }
}

/// In-process vector search. No daemon, no socket, no IPC.
///
/// * `handle`     — from `synapse_open`
/// * `query`      — pointer to `dim` f32 values (normalized)
/// * `dim`        — embedding dimension (must match index, typically 384)
/// * `k`          — results to return
/// * `mode`       — 1=binary_first 2=strict 3=binary_only
/// * `out_ids`    — caller-allocated i64 array of length k
/// * `out_scores` — caller-allocated f32 array of length k
///
/// Returns number of results written, or -1 on error.
///
/// # Safety
/// `handle`, `query`, `out_ids`, and `out_scores` must be valid for the
/// provided lengths. `out_ids` and `out_scores` must each have capacity for
/// at least `k` elements.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn synapse_search_raw(
    handle: *const SynapseHandle,
    query: *const f32,
    dim: usize,
    k: usize,
    mode: u8,
    out_ids: *mut i64,
    out_scores: *mut f32,
) -> i32 {
    if handle.is_null() || query.is_null() || out_ids.is_null() || out_scores.is_null() || k == 0 {
        return -1;
    }
    let q = unsafe { std::slice::from_raw_parts(query, dim) };
    let h = unsafe { &*handle };
    let g = h.index.load();
    let hits = match mode {
        2 => g.search_strict(q, k),
        3 => g.search_binary_only(q, k),
        _ => g.search_binary_first(q, k),
    };
    let n = hits.len().min(k);
    unsafe {
        for (i, hit) in hits.iter().take(n).enumerate() {
            *out_ids.add(i) = hit.id;
            *out_scores.add(i) = hit.score;
        }
    }
    n as i32
}
