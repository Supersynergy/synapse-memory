//! Stable C ABI for synapse-market.

use std::ffi::CStr;
use std::os::raw::c_char;
use std::ptr;

use crate::Market;

pub struct MarketHandle {
    inner: Market,
}

#[no_mangle]
pub extern "C" fn smx_market_open(path: *const c_char) -> *mut MarketHandle {
    if path.is_null() { return ptr::null_mut(); }
    let s = unsafe {
        match CStr::from_ptr(path).to_str() { Ok(s) => s, Err(_) => return ptr::null_mut() }
    };
    match Market::open(s) {
        Ok(m) => Box::into_raw(Box::new(MarketHandle { inner: m })),
        Err(_) => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn smx_market_close(handle: *mut MarketHandle) {
    if !handle.is_null() { unsafe { drop(Box::from_raw(handle)) }; }
}

#[no_mangle]
pub extern "C" fn smx_ingest_ohlcv(
    handle: *mut MarketHandle,
    ticker: *const c_char,
    rows: *const f64,
    n_rows: usize,
) -> i32 {
    if handle.is_null() || ticker.is_null() || rows.is_null() { return -1; }
    let sym = unsafe {
        match CStr::from_ptr(ticker).to_str() { Ok(s) => s, Err(_) => return -1 }
    };
    let raw = unsafe { std::slice::from_raw_parts(rows, n_rows * 6) };
    let parsed: Vec<(i64, f64, f64, f64, f64, f64)> = raw
        .chunks_exact(6)
        .map(|c| (c[0] as i64, c[1], c[2], c[3], c[4], c[5]))
        .collect();
    let m = unsafe { &*handle };
    match m.inner.ingest_ohlcv(sym, &parsed) { Ok(_) => 0, Err(_) => -1 }
}

#[no_mangle]
pub extern "C" fn smx_series_range_close(
    handle: *mut MarketHandle,
    ticker: *const c_char,
    start_ts: i64,
    end_ts: i64,
    out_buf: *mut f64,
    max_len: usize,
) -> i64 {
    if handle.is_null() || ticker.is_null() || out_buf.is_null() { return -1; }
    let sym = unsafe {
        match CStr::from_ptr(ticker).to_str() { Ok(s) => s, Err(_) => return -1 }
    };
    let m = unsafe { &*handle };
    let rows = match smx_query_range(&m.inner, sym, start_ts, end_ts) {
        Ok(r) => r, Err(_) => return -1,
    };
    let out = unsafe { std::slice::from_raw_parts_mut(out_buf, max_len) };
    let n = rows.len().min(max_len);
    for (i, row) in rows.iter().take(n).enumerate() { out[i] = row.4; }
    n as i64
}

pub fn smx_query_range(
    market: &Market,
    ticker: &str,
    start_ts: i64,
    end_ts: i64,
) -> crate::Result<Vec<(i64, f64, f64, f64, f64, f64)>> {
    let table = format!("ohlcv_{}", ticker.to_ascii_uppercase());
    let mut stmt = market.conn.prepare_cached(&format!(
        "SELECT ts, open, high, low, close, volume FROM \"{}\" WHERE ts >= ?1 AND ts <= ?2 ORDER BY ts",
        table
    ))?;
    let rows = stmt
        .query_map(rusqlite::params![start_ts, end_ts], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?, r.get::<_, f64>(2)?,
                r.get::<_, f64>(3)?, r.get::<_, f64>(4)?, r.get::<_, f64>(5)?))
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}
