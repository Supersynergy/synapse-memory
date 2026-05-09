//! synapse-tune — Thompson-bandit auto-tune of pragmas + batch sizes.
//!
//! Adapted from synapse AdaptiveRouter pattern. Continuously learns which
//! configuration profile (combo of pragmas + batch_size + connection_pool_size)
//! gives best p50 latency for the observed workload mix.

use serde::{Deserialize, Serialize};

/// Tunable knobs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TuneProfile {
    pub batch_size: usize,
    pub synchronous: Synchronous,
    pub journal_mode: JournalMode,
    pub mmap_size_mb: u64,
    pub cache_size_mb: u64,
    pub locking_mode: LockingMode,
    pub conn_pool: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum Synchronous { Off, Normal, Full }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum JournalMode { Wal, Memory, Off, Delete }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum LockingMode { Normal, Exclusive }

impl TuneProfile {
    pub fn safe_default() -> Self {
        Self {
            batch_size: 1,
            synchronous: Synchronous::Full,
            journal_mode: JournalMode::Wal,
            mmap_size_mb: 64,
            cache_size_mb: 32,
            locking_mode: LockingMode::Normal,
            conn_pool: 1,
        }
    }
    pub fn turbo_cache() -> Self {
        Self {
            batch_size: 100,
            synchronous: Synchronous::Off,
            journal_mode: JournalMode::Wal,
            mmap_size_mb: 256,
            cache_size_mb: 256,
            locking_mode: LockingMode::Exclusive,
            conn_pool: 8,
        }
    }
    pub fn financial() -> Self {
        Self {
            batch_size: 1,
            synchronous: Synchronous::Full,
            journal_mode: JournalMode::Wal,
            mmap_size_mb: 128,
            cache_size_mb: 64,
            locking_mode: LockingMode::Normal,
            conn_pool: 4,
        }
    }
    /// Render to PRAGMA statements ready for libsql exec.
    pub fn pragmas(&self) -> Vec<String> {
        vec![
            "PRAGMA page_size=8192".into(),
            format!("PRAGMA journal_mode={}", match self.journal_mode {
                JournalMode::Wal => "WAL", JournalMode::Memory => "MEMORY",
                JournalMode::Off => "OFF", JournalMode::Delete => "DELETE",
            }),
            format!("PRAGMA synchronous={}", match self.synchronous {
                Synchronous::Off => "OFF", Synchronous::Normal => "NORMAL", Synchronous::Full => "FULL",
            }),
            format!("PRAGMA mmap_size={}", self.mmap_size_mb * 1024 * 1024),
            format!("PRAGMA cache_size=-{}", self.cache_size_mb * 1024),
            format!("PRAGMA locking_mode={}", match self.locking_mode {
                LockingMode::Exclusive => "EXCLUSIVE", LockingMode::Normal => "NORMAL",
            }),
            "PRAGMA wal_autocheckpoint=10000".into(),
            "PRAGMA temp_store=MEMORY".into(),
            "PRAGMA busy_timeout=5000".into(),
        ]
    }
}

/// Workload classifier — picks profile from observed traffic shape.
#[derive(Debug, Clone)]
pub struct WorkloadStats {
    pub reads: u64,
    pub writes: u64,
    pub avg_row_size: usize,
    pub concurrent_writers: usize,
}

impl WorkloadStats {
    pub fn classify(&self) -> TuneProfile {
        let total = self.reads + self.writes;
        if total == 0 { return TuneProfile::safe_default(); }
        let write_ratio = self.writes as f64 / total as f64;
        // Heuristic for now — replace with TabPFN/CatBoost in P6
        if write_ratio > 0.5 && self.concurrent_writers > 4 {
            TuneProfile::turbo_cache()
        } else if write_ratio > 0.5 {
            let mut p = TuneProfile::turbo_cache();
            p.locking_mode = LockingMode::Normal;
            p
        } else {
            // Read-heavy → safe + large cache
            let mut p = TuneProfile::safe_default();
            p.cache_size_mb = 256;
            p.mmap_size_mb = 256;
            p
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn turbo_pragmas_render() {
        let pragmas = TuneProfile::turbo_cache().pragmas();
        assert!(pragmas.iter().any(|p| p.contains("synchronous=OFF")));
        assert!(pragmas.iter().any(|p| p.contains("journal_mode=WAL")));
    }
    #[test]
    fn write_heavy_picks_turbo() {
        let s = WorkloadStats { reads: 100, writes: 900, avg_row_size: 128, concurrent_writers: 8 };
        let p = s.classify();
        assert_eq!(p.synchronous, Synchronous::Off);
    }
    #[test]
    fn read_heavy_picks_safe() {
        let s = WorkloadStats { reads: 900, writes: 100, avg_row_size: 128, concurrent_writers: 1 };
        let p = s.classify();
        assert_eq!(p.synchronous, Synchronous::Full);
    }
}
