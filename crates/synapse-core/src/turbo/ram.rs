//! RAM optimization utilities: mlock, madvise, huge-pages.
//!
//! All ops are best-effort — errors logged, never fatal.
//! Feature gates:
//!   `mlock`      — pin memory pages, prevent swap (needs sufficient RLIMIT_MEMLOCK).
//!   `huge-pages` — Linux MAP_HUGETLB 2 MB pages for posting-list mmap (Linux only).

// ── madvise helpers ──────────────────────────────────────────────────────────

/// Hint: kernel will need these pages soon (prefetch from disk).
/// Call after mmap open on tantivy index files.
#[cfg(unix)]
pub fn madvise_willneed(ptr: *mut u8, len: usize) {
    if len == 0 {
        return;
    }
    unsafe {
        let ret = libc::madvise(ptr as *mut libc::c_void, len, libc::MADV_WILLNEED);
        if ret != 0 {
            tracing::debug!(
                "madvise(WILLNEED, {len}) failed: {}",
                std::io::Error::last_os_error()
            );
        }
    }
}

/// Hint: access will be sequential (log-replay, bulk-scan).
#[cfg(unix)]
pub fn madvise_sequential(ptr: *mut u8, len: usize) {
    if len == 0 {
        return;
    }
    unsafe {
        let ret = libc::madvise(ptr as *mut libc::c_void, len, libc::MADV_SEQUENTIAL);
        if ret != 0 {
            tracing::debug!("madvise(SEQUENTIAL, {len}) failed");
        }
    }
}

/// Hint: access will be random (HNSW graph traversal).
#[cfg(unix)]
pub fn madvise_random(ptr: *mut u8, len: usize) {
    if len == 0 {
        return;
    }
    unsafe {
        let ret = libc::madvise(ptr as *mut libc::c_void, len, libc::MADV_RANDOM);
        if ret != 0 {
            tracing::debug!("madvise(RANDOM, {len}) failed");
        }
    }
}

// No-ops on non-unix (Windows build compat).
#[cfg(not(unix))]
pub fn madvise_willneed(_ptr: *mut u8, _len: usize) {}
#[cfg(not(unix))]
pub fn madvise_sequential(_ptr: *mut u8, _len: usize) {}
#[cfg(not(unix))]
pub fn madvise_random(_ptr: *mut u8, _len: usize) {}

// ── mlock helper ─────────────────────────────────────────────────────────────

/// Pin `[ptr, ptr+len)` into RAM — prevents swap eviction of hot HNSW vectors.
/// Feature-gated `mlock` (off by default — needs RLIMIT_MEMLOCK or CAP_IPC_LOCK).
/// Returns true if successful.
#[cfg(all(feature = "mlock", unix))]
pub fn mlock_region(ptr: *mut u8, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    let ret = unsafe { libc::mlock(ptr as *const libc::c_void, len) };
    if ret != 0 {
        tracing::warn!("mlock({len} bytes) failed — check RLIMIT_MEMLOCK (ulimit -l)");
        false
    } else {
        tracing::info!("mlock: pinned {} MiB in RAM", len / (1024 * 1024));
        true
    }
}

#[cfg(not(all(feature = "mlock", unix)))]
pub fn mlock_region(_ptr: *mut u8, _len: usize) -> bool {
    false
}

// ── huge-pages mmap (Linux only) ─────────────────────────────────────────────

/// Allocate `len` bytes backed by 2 MB huge-pages via MAP_HUGETLB.
/// Returns `None` if the kernel lacks huge-page support or `len` == 0.
/// Caller must free via `munmap_huge`.
///
/// Feature-gated `huge-pages` — Linux only. macOS uses transparent huge-pages
/// automatically; explicit MAP_HUGETLB does not exist there.
#[cfg(all(feature = "huge-pages", target_os = "linux"))]
pub fn mmap_huge(len: usize) -> Option<*mut u8> {
    if len == 0 {
        return None;
    }
    // Round up to 2 MiB boundary.
    const HUGE: usize = 2 * 1024 * 1024;
    let aligned = (len + HUGE - 1) & !(HUGE - 1);
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            aligned,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_HUGETLB,
            -1,
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        tracing::warn!("mmap(MAP_HUGETLB, {aligned} bytes) failed — falling back to normal pages");
        None
    } else {
        tracing::info!(
            "huge-pages: allocated {} MiB via MAP_HUGETLB",
            aligned / (1024 * 1024)
        );
        Some(ptr as *mut u8)
    }
}

/// # Safety: ptr must have been returned by `mmap_huge` with the same `len`.
#[cfg(all(feature = "huge-pages", target_os = "linux"))]
pub unsafe fn munmap_huge(ptr: *mut u8, len: usize) {
    const HUGE: usize = 2 * 1024 * 1024;
    let aligned = (len + HUGE - 1) & !(HUGE - 1);
    libc::munmap(ptr as *mut libc::c_void, aligned);
}

// ── thread-local query buffer ─────────────────────────────────────────────────

/// Per-thread reusable float buffer for query vectors — avoids one heap alloc per query.
///
/// Usage:
/// ```ignore
/// use synapse_core::turbo::ram::with_query_buf;
/// with_query_buf(dim, |buf| {
///     buf.copy_from_slice(&embedding);
///     // … use buf …
/// });
/// ```
pub fn with_query_buf<F, R>(dim: usize, f: F) -> R
where
    F: FnOnce(&mut Vec<f32>) -> R,
{
    thread_local! {
        static QUERY_BUF: std::cell::RefCell<Vec<f32>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    QUERY_BUF.with(|cell| {
        let mut buf = cell.borrow_mut();
        if buf.len() < dim {
            buf.resize(dim, 0.0);
        }
        // Zero only the active slice to avoid stale data.
        buf[..dim].fill(0.0);
        f(&mut buf)
    })
}

// ── warm-cache: touch mmap pages at startup ──────────────────────────────────

/// Walk every file under `dir`, open it, and read it sequentially so the OS
/// pulls pages into the page-cache before the first query hits them.
/// Runs in a background thread — returns immediately.
pub fn prefetch_dir_bg(dir: std::path::PathBuf) {
    std::thread::Builder::new()
        .name("synapse-prefetch".into())
        .spawn(move || {
            let start = std::time::Instant::now();
            let mut total_bytes: u64 = 0;
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file()
                        && let Ok(mut f) = std::fs::File::open(&path)
                    {
                        use std::io::Read;
                        let mut sink = [0u8; 65536];
                        loop {
                            match f.read(&mut sink) {
                                Ok(0) | Err(_) => break,
                                Ok(n) => total_bytes += n as u64,
                            }
                        }
                    }
                }
            }
            tracing::info!(
                "prefetch_dir_bg: {} MiB in {:.1}s",
                total_bytes / (1024 * 1024),
                start.elapsed().as_secs_f32()
            );
        })
        .ok();
}
