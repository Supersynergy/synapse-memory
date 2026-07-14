/// C-ABI surface — Day 1 stubs + Phase 10 Day 5 RRF kernel + Day 7 obfstr hardening.
#[unsafe(no_mangle)]
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
#[unsafe(no_mangle)]
pub extern "C" fn synapse_engine_score(query: *const u8, query_len: usize, top_k: u32) -> i32 {
    let _ = (query, query_len, top_k);
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn synapse_engine_version() -> *const u8 {
    static VERSION_STR: &str = "synapse-engine-v0.1.0\0";
    VERSION_STR.as_ptr()
}

/// Returns obfuscated version string at runtime (heap-allocated, not a literal).
#[unsafe(no_mangle)]
pub extern "C" fn synapse_engine_version_obf() -> *const u8 {
    let s = obfstr::obfstr!("synapse-engine-v0.1.0").to_owned();
    let b = s.into_bytes().into_boxed_slice();
    Box::into_raw(b) as *const u8
}

/// RRF fusion over two rank lists.
///
/// Writes fused scores into `out_ptr[0..n]` where n = min(a_len.max(b_len), out_cap).
/// Returns the number of elements written, or -1 on invalid input.
///
/// # Safety
///
/// `a_ptr` must point to `a_len` valid `f64` values (or be null when `a_len == 0`).
/// `b_ptr` must point to `b_len` valid `f64` values (or be null when `b_len == 0`).
/// `out_ptr` must point to a writable buffer of at least `out_cap` `f64` values.
#[unsafe(no_mangle)]
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
    let a = unsafe { std::slice::from_raw_parts(a_ptr, a_len) };
    let b = unsafe { std::slice::from_raw_parts(b_ptr, b_len) };
    let scored = crate::rrf::rrf_fuse(a, b, k);
    let n = scored.len().min(out_cap);
    unsafe {
        std::ptr::copy_nonoverlapping(scored.as_ptr(), out_ptr, n);
    }
    n as i32
}
