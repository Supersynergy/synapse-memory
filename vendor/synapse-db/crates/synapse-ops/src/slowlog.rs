//! Slow query log — records queries above duration threshold.
//!
//! Closes "no slow query log" gap. Top-N analysis simpler than MySQL pt-query-digest.

use serde::{Deserialize, Serialize};
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlowEntry {
    pub sql: String,
    pub duration_us: u64,
    pub timestamp_unix: u64,
}

pub struct SlowQueryLog {
    threshold_us: AtomicU64,
    entries: RwLock<Vec<SlowEntry>>,
    cap: usize,
}

impl SlowQueryLog {
    pub fn new(threshold: Duration, cap: usize) -> Self {
        Self {
            threshold_us: AtomicU64::new(threshold.as_micros() as u64),
            entries: RwLock::new(Vec::with_capacity(cap.min(10_000))),
            cap,
        }
    }

    pub fn record(&self, sql: &str, duration: Duration) {
        let us = duration.as_micros() as u64;
        if us < self.threshold_us.load(Ordering::Relaxed) {
            return;
        }
        let entry = SlowEntry {
            sql: sql.to_string(),
            duration_us: us,
            timestamp_unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        };
        if let Ok(mut g) = self.entries.write() {
            if g.len() >= self.cap {
                g.remove(0);
            }
            g.push(entry);
        }
    }

    pub fn top_n(&self, n: usize) -> Vec<SlowEntry> {
        let g = self.entries.read().unwrap();
        let mut v: Vec<SlowEntry> = g.iter().cloned().collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.duration_us));
        v.truncate(n);
        v
    }

    pub fn set_threshold(&self, threshold: Duration) {
        self.threshold_us
            .store(threshold.as_micros() as u64, Ordering::Relaxed);
    }

    pub fn len(&self) -> usize {
        self.entries.read().map(|g| g.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.read().map(|g| g.is_empty()).unwrap_or(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slow_log_records_above_threshold() {
        let log = SlowQueryLog::new(Duration::from_millis(10), 100);
        log.record("SELECT 1", Duration::from_millis(20));
        log.record("SELECT 2", Duration::from_millis(5));
        assert_eq!(log.len(), 1);
        let top = log.top_n(10);
        assert_eq!(top[0].sql, "SELECT 1");
    }
    #[test]
    fn top_n_sorts_by_duration() {
        let log = SlowQueryLog::new(Duration::from_micros(0), 100);
        log.record("fast", Duration::from_micros(5));
        log.record("slow", Duration::from_millis(50));
        log.record("medium", Duration::from_millis(10));
        let top = log.top_n(3);
        assert_eq!(top[0].sql, "slow");
        assert_eq!(top[1].sql, "medium");
        assert_eq!(top[2].sql, "fast");
    }
    #[test]
    fn cap_evicts_oldest() {
        let log = SlowQueryLog::new(Duration::from_micros(0), 2);
        log.record("a", Duration::from_micros(1));
        log.record("b", Duration::from_micros(1));
        log.record("c", Duration::from_micros(1));
        assert_eq!(log.len(), 2);
    }
}
