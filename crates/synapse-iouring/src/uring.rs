//! Real io_uring WAL writer — Linux only, enabled by `--features io-uring`.
//!
//! Pattern (TigerBeetle):
//!   1. Open WAL file with O_DIRECT | O_CREAT | O_WRONLY | O_APPEND
//!   2. Batch 32+ write SQEs → io_uring_enter (single syscall)
//!   3. Drain CQEs — propagate first error
//!
//! Direct-I/O constraint: all buffers must be 512-byte aligned, length % 512 == 0.
//! We satisfy this with a memaligned buffer-pool (Layout::from_size_align 512).

use crate::error::{IoUringError, Result};
use crate::lsm::Entry;
use crossbeam_queue::ArrayQueue;
use io_uring::{IoUring, opcode, types};
use std::alloc::{Layout, alloc, dealloc};
use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::Path;
use std::sync::Arc;

const SECTOR: usize = 512;
const BUF_SIZE: usize = 4096; // 4 KB, power-of-two, 512-aligned
const POOL_CAP: usize = 64;
const RING_SZ: u32 = 256;

/// A single memaligned buffer. Drop returns it to the pool.
struct AlignedBuf {
    ptr: *mut u8,
    layout: Layout,
    len: usize, // bytes actually written (≤ BUF_SIZE)
    pool: Arc<ArrayQueue<AlignedBuf>>,
}

// Safety: ptr is exclusively owned while borrowed from pool.
unsafe impl Send for AlignedBuf {}

impl AlignedBuf {
    fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        if self.ptr.is_null() {
            return;
        }
        let ptr = std::mem::replace(&mut self.ptr, std::ptr::null_mut());
        // Safety: pool capacity == POOL_CAP, we always pop before push,
        // so push never fails. Use ManuallyDrop to hand off ownership.
        let buf = AlignedBuf { ptr, layout: self.layout, len: 0, pool: self.pool.clone() };
        if let Err(mut rejected) = self.pool.push(buf) {
            // Pool full (shouldn't happen). Null out ptr so recursive Drop is a no-op.
            let rptr = std::mem::replace(&mut rejected.ptr, std::ptr::null_mut());
            unsafe { dealloc(rptr, rejected.layout) };
            // `rejected` drops here; ptr is null so Drop returns early.
        }
    }
}

fn make_pool() -> Arc<ArrayQueue<AlignedBuf>> {
    let pool = Arc::new(ArrayQueue::new(POOL_CAP));
    let layout = Layout::from_size_align(BUF_SIZE, SECTOR).expect("valid layout");
    for _ in 0..POOL_CAP {
        let ptr = unsafe { alloc(layout) };
        assert!(!ptr.is_null(), "memalign alloc failed");
        let buf = AlignedBuf { ptr, layout, len: 0, pool: pool.clone() };
        pool.push(buf).unwrap_or_else(|_| panic!("pool push failed"));
    }
    pool
}

pub struct UringWal {
    ring: IoUring,
    fd: RawFd,
    offset: u64,
    batch_sz: usize,
    pool: Arc<ArrayQueue<AlignedBuf>>,
}

impl UringWal {
    pub fn open(path: &Path, batch_sz: usize) -> Result<Self> {
        // Try O_DIRECT first (native ext4/xfs). Fall back to buffered on overlay/tmpfs.
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .custom_flags(libc::O_DIRECT)
            .open(path)
            .or_else(|_| {
                tracing::warn!("O_DIRECT unsupported on this fs — falling back to buffered I/O");
                OpenOptions::new().create(true).write(true).open(path)
            })
            .map_err(|e| IoUringError::SetupFailed(format!("WAL open: {e}")))?;

        let fd = file.as_raw_fd();
        std::mem::forget(file);

        let ring = IoUring::new(RING_SZ)
            .map_err(|e| IoUringError::SetupFailed(format!("io_uring_setup: {e}")))?;

        let pool = make_pool();

        Ok(Self { ring, fd, offset: 0, batch_sz, pool })
    }

    /// Borrow a buffer from pool, copy data in (padded to SECTOR boundary).
    fn acquire_buf(&self, data: &[u8]) -> AlignedBuf {
        let mut buf = self.pool.pop().expect("pool exhausted — increase POOL_CAP");
        let padded = align_up(data.len(), SECTOR);
        assert!(padded <= BUF_SIZE, "entry too large for 4KB buffer");
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), buf.ptr, data.len());
            // zero-pad remainder for O_DIRECT
            std::ptr::write_bytes(buf.ptr.add(data.len()), 0, padded - data.len());
        }
        buf.len = padded;
        buf
    }

    /// Write entries in batches of `batch_sz` SQEs.
    pub async fn write_batch(&mut self, entries: &[Entry]) -> Result<()> {
        for chunk in entries.chunks(self.batch_sz) {
            // Serialize + acquire memaligned buffers
            let raw_bufs: Vec<Vec<u8>> = chunk
                .iter()
                .map(|e| serde_json::to_vec(e).expect("Entry serialization infallible"))
                .collect();

            let aligned: Vec<AlignedBuf> = raw_bufs
                .iter()
                .map(|raw| self.acquire_buf(raw))
                .collect();

            // Submit SQEs
            {
                let mut sq = self.ring.submission();
                for buf in &aligned {
                    let write_e = opcode::Write::new(
                        types::Fd(self.fd),
                        buf.ptr,
                        buf.len as u32,
                    )
                    .offset(self.offset)
                    .build();
                    unsafe { sq.push(&write_e) }
                        .map_err(|_| IoUringError::AppendFailed("SQ full".into()))?;
                    self.offset += buf.len as u64;
                }
            }

            // Single syscall: io_uring_enter
            self.ring
                .submit_and_wait(chunk.len())
                .map_err(|e| IoUringError::AppendFailed(format!("submit_and_wait: {e}")))?;

            // Drain CQEs — propagate first error
            let cq = self.ring.completion();
            for cqe in cq {
                if cqe.result() < 0 {
                    return Err(IoUringError::AppendFailed(format!(
                        "CQE error: {} (errno {})",
                        cqe.result(),
                        -cqe.result()
                    )));
                }
            }
            // aligned drops here → buffers return to pool
        }
        Ok(())
    }
}

#[inline]
fn align_up(n: usize, align: usize) -> usize {
    (n + align - 1) & !(align - 1)
}
