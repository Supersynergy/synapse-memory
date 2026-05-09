//! Pragma auto-tuner. Heuristic-baseline today, TabPFN-2.5 wiring planned via PyO3.
//!
//! Input: WorkloadStats (read/write ratio, concurrency, row sizes).
//! Output: TuneProfile (9 PRAGMA values + batch_size + pool_size).
//!
//! Heuristic heute = decision-tree von handgewählten thresholds.
//! Bench-validated against measured workloads. Replace mit TabPFN inference
//! sobald Python sidecar exists.

use crate::{TuneProfile, WorkloadStats, Synchronous, JournalMode, LockingMode};

/// Tuner trait — different impls (heuristic, TabPFN, Optuna).
pub trait Tuner: Send + Sync {
    fn name(&self) -> &'static str;
    fn pick(&self, stats: &WorkloadStats) -> TuneProfile;
}

pub struct HeuristicTuner;

impl Tuner for HeuristicTuner {
    fn name(&self) -> &'static str { "heuristic-v1" }

    fn pick(&self, stats: &WorkloadStats) -> TuneProfile {
        let total = stats.reads + stats.writes;
        if total == 0 { return TuneProfile::safe_default(); }
        let write_ratio = stats.writes as f64 / total as f64;
        let cw = stats.concurrent_writers;
        let rs = stats.avg_row_size;

        // Decision tree (calibratable from autolearn race results)
        match (write_ratio, cw, rs) {
            // High write + high concurrency → turbo + large pool
            (w, c, _) if w > 0.5 && c > 4 => {
                let mut p = TuneProfile::turbo_cache();
                p.conn_pool = c.min(32);
                p.batch_size = if w > 0.8 { 200 } else { 100 };
                p
            }
            // High write + low concurrency → turbo + smaller pool
            (w, _, _) if w > 0.5 => {
                let mut p = TuneProfile::turbo_cache();
                p.locking_mode = LockingMode::Normal;
                p.conn_pool = 4;
                p
            }
            // Mixed
            (w, c, _) if w > 0.2 => {
                let mut p = TuneProfile::safe_default();
                p.synchronous = Synchronous::Normal;
                p.cache_size_mb = 256;
                p.mmap_size_mb = 256;
                p.conn_pool = c.min(16).max(4);
                p
            }
            // Read-heavy → big cache, full durability
            _ => {
                let mut p = TuneProfile::safe_default();
                p.cache_size_mb = if rs > 1024 { 512 } else { 256 };
                p.mmap_size_mb = 256;
                p.conn_pool = stats.concurrent_writers.max(8);
                p
            }
        }
    }
}

/// Stub for TabPFN/Optuna integration (PyO3 sidecar — P3).
pub struct TabPfnTuner;

impl Tuner for TabPfnTuner {
    fn name(&self) -> &'static str { "tabpfn-stub" }
    fn pick(&self, stats: &WorkloadStats) -> TuneProfile {
        // TODO P3: PyO3 → call tabpfn-classifier with stats features
        // Returns one-hot over TuneProfile presets.
        // For now fallback to heuristic.
        HeuristicTuner.pick(stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(r: u64, w: u64, c: usize, s: usize) -> WorkloadStats {
        WorkloadStats { reads: r, writes: w, avg_row_size: s, concurrent_writers: c }
    }

    #[test]
    fn write_heavy_concurrent_picks_turbo_large_pool() {
        let p = HeuristicTuner.pick(&stats(100, 900, 16, 128));
        assert_eq!(p.synchronous, Synchronous::Off);
        assert_eq!(p.locking_mode, LockingMode::Exclusive);
        assert_eq!(p.conn_pool, 16);
        assert_eq!(p.batch_size, 200);
    }
    #[test]
    fn write_heavy_low_concurrent_normal_locking() {
        let p = HeuristicTuner.pick(&stats(100, 800, 2, 128));
        assert_eq!(p.locking_mode, LockingMode::Normal);
        assert_eq!(p.conn_pool, 4);
    }
    #[test]
    fn read_heavy_picks_big_cache() {
        let p = HeuristicTuner.pick(&stats(900, 100, 4, 2048));
        assert_eq!(p.synchronous, Synchronous::Full);
        assert_eq!(p.cache_size_mb, 512);
    }
    #[test]
    fn empty_stats_safe_default() {
        let p = HeuristicTuner.pick(&stats(0, 0, 0, 0));
        assert_eq!(p, TuneProfile::safe_default());
    }
}
