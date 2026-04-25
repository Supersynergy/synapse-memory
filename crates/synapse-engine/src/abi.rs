/// C-ABI surface — Day 1 stubs + Phase 10 Day 5 RRF kernel.

#[no_mangle]
pub extern "C" fn synapse_engine_init(db_path: *const u8, db_path_len: usize) -> i32 {
    let _ = (db_path, db_path_len);
    0
}

/// Score a query against the engine index.
///
/// `query`     — UTF-8 bytes, not null-terminated
/// `query_len` — byte length of query
/// `top_k`     — number of results requested
///
/// Returns 0 on success, negative errno on error.
#[no_mangle]
pub extern "C" fn synapse_engine_score(
    query: *const u8,
    query_len: usize,
    top_k: u32,
) -> i32 {
    let _ = (query, query_len, top_k);
    0
}

#[no_mangle]
pub extern "C" fn synapse_engine_version() -> *const u8 {
    b"synapse-engine-v0.1.0\0".as_ptr()
}

/// RRF fusion over two rank lists.
///
/// Writes fused scores into `out_ptr[0..n]` where n = min(a_len.max(b_len), out_cap).
/// Returns the number of elements written, or -1 on invalid input.
#[no_mangle]
pub unsafe extern "C" fn synapse_engine_rrf_fuse(
    a_ptr: *const f64,
    a_len: usize,
    b_ptr: *const f64,
    b_len: usize,
    k: f64,
    out_ptr: *mut f64,
    out_cap: usize,
) -> i32 {
    if (a_len > 0 && a_ptr.is_null()) || (b_len > 0 && b_ptr.is_null()) || out_ptr.is_null() {
        return -1;
    }
    let a = std::slice::from_raw_parts(a_ptr, a_len);
    let b = std::slice::from_raw_parts(b_ptr, b_len);
    let scored = crate::rrf::rrf_fuse(a, b, k);
    let n = scored.len().min(out_cap);
    std::ptr::copy_nonoverlapping(scored.as_ptr(), out_ptr, n);
    n as i32
}
