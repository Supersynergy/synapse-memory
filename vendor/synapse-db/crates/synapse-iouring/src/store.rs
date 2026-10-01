//! `IoUringStore` — main public API.
//!
//! On Linux with `--features io-uring`: uses real io_uring WAL writes.
//! On macOS / Windows: all methods compile but return `UnsupportedPlatform` at runtime.

#[allow(unused_imports)]
use crate::error::{IoUringError, Result};
#[allow(unused_imports)]
use crate::lsm::{Entry, Key, L0, SSTable};
#[allow(unused_imports)]
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[allow(unused_imports)]
use std::sync::atomic::{AtomicU64, Ordering};
#[allow(unused_imports)]
use tracing::{debug, info, warn};

/// Batch size: number of io_uring SQEs submitted per syscall.
#[cfg(feature = "io-uring")]
const BATCH_SZ: usize = 32;

/// L0 flush threshold (entries).
const L0_FLUSH: usize = 4096;

#[allow(dead_code)]
pub struct IoUringStore {
    dir: PathBuf,
    l0: Arc<L0>,
    sstables: Vec<SSTable>,
    seq: AtomicU64,

    #[cfg(feature = "io-uring")]
    uring: crate::uring::UringWal,
}

impl IoUringStore {
    /// Open (or create) a store at `dir`.
    ///
    /// # Errors
    /// - `UnsupportedPlatform` on non-Linux builds without `io-uring` feature.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;

        #[cfg(feature = "io-uring")]
        {
            let wal_path = dir.join("WAL");
            let uring = crate::uring::UringWal::open(&wal_path, BATCH_SZ)?;
            info!(path = ?dir, "IoUringStore opened (io_uring mode)");
            return Ok(Self {
                dir,
                l0: Arc::new(L0::new(L0_FLUSH)),
                sstables: Vec::new(),
                seq: AtomicU64::new(0),
                uring,
            });
        }

        #[cfg(not(feature = "io-uring"))]
        {
            warn!("io_uring feature disabled — store opened in no-op mode");
            Ok(Self {
                dir,
                l0: Arc::new(L0::new(L0_FLUSH)),
                sstables: Vec::new(),
                seq: AtomicU64::new(0),
            })
        }
    }

    /// Append a batch of entries via io_uring WAL.
    ///
    /// Batches are submitted in groups of `BATCH_SZ` SQEs (TigerBeetle pattern).
    /// After WAL, entries are inserted into L0.  When L0 hits the flush threshold,
    /// an SSTable is written synchronously (async compaction: future work).
    pub async fn append_batch(&mut self, entries: Vec<Entry>) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }

        #[cfg(not(feature = "io-uring"))]
        return Err(IoUringError::UnsupportedPlatform);

        #[cfg(feature = "io-uring")]
        {
            // Assign sequence numbers
            let mut stamped: Vec<Entry> = entries
                .into_iter()
                .map(|mut e| {
                    e.seq = self.seq.fetch_add(1, Ordering::Relaxed);
                    e
                })
                .collect();

            // WAL write via io_uring (batched)
            self.uring.write_batch(&stamped).await?;

            // Insert into L0
            let mut needs_flush = false;
            for e in &stamped {
                if self.l0.insert(e.clone()) {
                    needs_flush = true;
                }
            }

            if needs_flush {
                self.flush_l0()?;
            }

            debug!(count = stamped.len(), "append_batch ok");
            Ok(())
        }
    }

    /// Append with explicit durability control (TigerBeetle submit_link pattern).
    ///
    /// - `Durability::Fast`    — writes only, no fsync (~5-10M/s expected on Linux NVMe)
    /// - `Durability::Batched` — N writes + 1 fsync via SQE_LINK chain (~100k-500k/s durable)
    /// - `Durability::Strict`  — 1 write + 1 fsync per row (~100-10k/s, per-row durable)
    pub async fn batched_append(
        &mut self,
        entries: Vec<Entry>,
        #[cfg(feature = "io-uring")] durability: crate::uring::Durability,
        #[cfg(not(feature = "io-uring"))] _durability: (),
    ) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }

        #[cfg(not(feature = "io-uring"))]
        return Err(IoUringError::UnsupportedPlatform);

        #[cfg(feature = "io-uring")]
        {
            let stamped: Vec<Entry> = entries
                .into_iter()
                .map(|mut e| {
                    e.seq = self.seq.fetch_add(1, Ordering::Relaxed);
                    e
                })
                .collect();

            self.uring
                .batched_append(stamped.clone(), durability)
                .await?;

            let mut needs_flush = false;
            for e in &stamped {
                if self.l0.insert(e.clone()) {
                    needs_flush = true;
                }
            }
            if needs_flush {
                self.flush_l0()?;
            }
            Ok(())
        }
    }

    /// Range scan: L0 + SSTables (L0 wins on conflict by higher seq).
    pub async fn read_range(&self, _key_range: Range<Key>) -> Result<Vec<Entry>> {
        #[cfg(not(feature = "io-uring"))]
        return Err(IoUringError::UnsupportedPlatform);

        #[cfg(feature = "io-uring")]
        {
            let mut out = self.l0.scan(&_key_range);
            // TODO: merge SSTable results (L1+ scan) — deduped by seq
            out.sort_by(|a, b| a.key.cmp(&b.key).then(b.seq.cmp(&a.seq)));
            out.dedup_by(|a, b| a.key == b.key); // keep highest seq (first after sort)
            Ok(out)
        }
    }

    #[allow(dead_code)]
    fn flush_l0(&mut self) -> Result<()> {
        let mut entries = self.l0.drain();
        entries.sort_by(|a, b| a.key.cmp(&b.key));
        let path = self
            .dir
            .join(format!("sst-{:016x}.sst", self.seq.load(Ordering::Relaxed)));
        let sst = SSTable::write(path, &entries)?;
        info!(entries = sst.entry_count, "L0 flushed → SSTable");
        self.sstables.push(sst);
        Ok(())
    }
}
