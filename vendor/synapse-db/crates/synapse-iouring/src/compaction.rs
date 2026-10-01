//! Tiered compaction worker.
//!
//! Strategy:
//!   L0 (memtable) → flush to SSTable on disk
//!   L1: up to 4 SSTables  → merge → L2 SSTable on full
//!   L2: up to 16 SSTables → merge → L3 SSTable on full
//!
//! Background task: `Compactor::spawn` returns a handle + command channel.

use crate::error::{IoUringError, Result};
use crate::lsm::{Entry, SSTable};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// Tiered level config.
#[derive(Debug, Clone)]
pub struct TieredConfig {
    /// Max SSTables in L1 before compacting to L2.
    pub l1_max: usize,
    /// Max SSTables in L2 before compacting to L3.
    pub l2_max: usize,
    /// Directory where SSTable files live.
    pub dir: PathBuf,
}

impl Default for TieredConfig {
    fn default() -> Self {
        Self {
            l1_max: 4,
            l2_max: 16,
            dir: PathBuf::from("."),
        }
    }
}

/// Commands sent to the compaction background task.
#[derive(Debug)]
pub enum CompactCmd {
    /// Flush L0 entries to a new SSTable (adds to L1).
    FlushL0(Vec<Entry>),
    /// Graceful shutdown.
    Shutdown,
}

pub struct Compactor {
    pub config: TieredConfig,
    /// Monotonic counter for SSTable file naming.
    seq: Arc<AtomicU64>,
    l1: Vec<SSTable>,
    l2: Vec<SSTable>,
}

impl Compactor {
    pub fn new(config: TieredConfig) -> Self {
        Self {
            config,
            seq: Arc::new(AtomicU64::new(0)),
            l1: Vec::new(),
            l2: Vec::new(),
        }
    }

    /// Spawn the compaction worker as a Tokio background task.
    ///
    /// Returns a sender; drop sender to signal shutdown.
    pub fn spawn(config: TieredConfig) -> mpsc::Sender<CompactCmd> {
        let (tx, rx) = mpsc::channel::<CompactCmd>(64);
        tokio::spawn(async move {
            let mut worker = Compactor::new(config);
            worker.run(rx).await;
        });
        tx
    }

    async fn run(&mut self, mut rx: mpsc::Receiver<CompactCmd>) {
        while let Some(cmd) = rx.recv().await {
            match cmd {
                CompactCmd::FlushL0(entries) => {
                    if let Err(e) = self.flush_l0(entries) {
                        warn!(err = %e, "compactor flush_l0 failed");
                    }
                }
                CompactCmd::Shutdown => {
                    info!("compactor shutdown");
                    break;
                }
            }
        }
    }

    /// Flush L0 entries → new SSTable in L1. Triggers L1→L2 compaction if full.
    fn flush_l0(&mut self, mut entries: Vec<Entry>) -> Result<()> {
        entries.sort_by(|a, b| a.key.cmp(&b.key));
        if entries.is_empty() {
            return Ok(());
        }

        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let path = self.config.dir.join(format!("L1-{:016x}.sst", seq));
        let sst = SSTable::write(path, &entries)?;
        info!(entries = sst.entry_count, level = 1, "flushed SSTable");
        self.l1.push(sst);

        if self.l1.len() >= self.config.l1_max {
            self.compact_level(1)?;
        }

        Ok(())
    }

    /// Merge all SSTables at `level` into a single SSTable at `level+1`.
    fn compact_level(&mut self, level: usize) -> Result<()> {
        let (src, dst) = match level {
            1 => (&mut self.l1, &mut self.l2),
            _ => {
                return Err(IoUringError::Compaction(format!(
                    "level {} not supported",
                    level
                )));
            }
        };

        let drained: Vec<SSTable> = std::mem::take(src);
        debug!(count = drained.len(), level, "merging SSTables");

        let mut all: Vec<Entry> = drained
            .iter()
            .flat_map(|sst| sst.read_entries(None).unwrap_or_default())
            .collect();

        // Merge sort + deduplicate by key (highest seq wins)
        all.sort_by(|a, b| a.key.cmp(&b.key).then(b.seq.cmp(&a.seq)));
        all.dedup_by(|a, b| a.key == b.key); // a is later = lower seq, b kept

        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let out_level = level + 1;
        let path = self
            .config
            .dir
            .join(format!("L{}-{:016x}.sst", out_level, seq));
        let merged = SSTable::write(path, &all)?;
        info!(
            entries = merged.entry_count,
            level = out_level,
            "compacted SSTable"
        );

        dst.push(merged);

        if out_level == 2 && dst.len() >= self.config.l2_max {
            warn!("L2 full — further compaction (L3) not yet implemented");
        }

        // Delete old SSTable files
        for sst in &drained {
            let _ = std::fs::remove_file(&sst.path);
            let bloom = sst.path.with_extension("sst.bloom");
            let _ = std::fs::remove_file(bloom);
        }

        Ok(())
    }
}
