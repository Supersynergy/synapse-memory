/// C-ABI surface — Day 1 stubs. Real IP migrated Phase 10 Day 5+.

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
