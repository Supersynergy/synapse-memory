//! Anomaly detection — EWMA + Welford online mean/variance.
//!
//! Detect query-latency drift in real-time. P99 spike → emit alert.
//! Replaces River (Python) with pure-Rust online stats.

use std::sync::RwLock;

#[derive(Debug, Clone, Copy)]
pub struct OnlineStats {
    n: u64,
    mean: f64,
    m2: f64,   // sum of squares of differences (Welford)
    ewma: f64, // exponentially-weighted moving average
    ewma_alpha: f64,
}

impl OnlineStats {
    pub fn new(ewma_alpha: f64) -> Self {
        Self {
            n: 0,
            mean: 0.0,
            m2: 0.0,
            ewma: 0.0,
            ewma_alpha,
        }
    }
    pub fn observe(&mut self, x: f64) {
        self.n += 1;
        let delta = x - self.mean;
        self.mean += delta / self.n as f64;
        let delta2 = x - self.mean;
        self.m2 += delta * delta2;
        if self.n == 1 {
            self.ewma = x;
        } else {
            self.ewma = self.ewma_alpha * x + (1.0 - self.ewma_alpha) * self.ewma;
        }
    }
    pub fn variance(&self) -> f64 {
        if self.n < 2 {
            0.0
        } else {
            self.m2 / (self.n - 1) as f64
        }
    }
    pub fn stddev(&self) -> f64 {
        self.variance().sqrt()
    }
    pub fn z_score(&self, x: f64) -> f64 {
        let s = self.stddev();
        if s < 1e-9 {
            0.0
        } else {
            (x - self.mean) / s
        }
    }
    pub fn ewma(&self) -> f64 {
        self.ewma
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnomalyVerdict {
    Normal,
    Anomaly { z_score_int: i32 },
}

pub struct DriftDetector {
    stats: RwLock<OnlineStats>,
    z_threshold: f64,
    min_samples: u64,
}

impl DriftDetector {
    pub fn new(z_threshold: f64, ewma_alpha: f64, min_samples: u64) -> Self {
        Self {
            stats: RwLock::new(OnlineStats::new(ewma_alpha)),
            z_threshold,
            min_samples,
        }
    }

    /// Observe metric. Returns verdict.
    pub fn check(&self, x: f64) -> AnomalyVerdict {
        let mut g = self.stats.write().unwrap();
        g.observe(x);
        if g.n < self.min_samples {
            return AnomalyVerdict::Normal;
        }
        let z = g.z_score(x);
        if z.abs() > self.z_threshold {
            AnomalyVerdict::Anomaly {
                z_score_int: z.round() as i32,
            }
        } else {
            AnomalyVerdict::Normal
        }
    }

    pub fn snapshot(&self) -> OnlineStats {
        *self.stats.read().unwrap()
    }
}

impl Default for DriftDetector {
    /// 3σ threshold, 0.1 EWMA alpha, warm-up 30 samples.
    fn default() -> Self {
        Self::new(3.0, 0.1, 30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn welford_mean_variance() {
        let mut s = OnlineStats::new(0.1);
        for x in [1.0, 2.0, 3.0, 4.0, 5.0] {
            s.observe(x);
        }
        assert!((s.mean - 3.0).abs() < 1e-9);
        assert!((s.variance() - 2.5).abs() < 1e-9);
    }

    #[test]
    fn drift_detects_3sigma_spike() {
        let d = DriftDetector::new(3.0, 0.1, 10);
        // Normal traffic ~50µs ± 5
        for _ in 0..50 {
            assert!(matches!(d.check(50.0), AnomalyVerdict::Normal));
        }
        // Suddenly 500µs spike (10× above mean)
        let v = d.check(500.0);
        assert!(matches!(v, AnomalyVerdict::Anomaly { .. }));
    }

    #[test]
    fn drift_no_alert_during_warmup() {
        let d = DriftDetector::new(2.0, 0.1, 100);
        for _ in 0..50 {
            assert!(matches!(d.check(1.0), AnomalyVerdict::Normal));
        }
        // Spike during warmup → no alert
        assert!(matches!(d.check(99999.0), AnomalyVerdict::Normal));
    }

    #[test]
    fn ewma_tracks_recent() {
        let mut s = OnlineStats::new(0.5);
        for _ in 0..10 {
            s.observe(100.0);
        }
        let pre = s.ewma();
        for _ in 0..10 {
            s.observe(200.0);
        }
        let post = s.ewma();
        assert!(post > pre);
        assert!(post > 150.0);
    }
}
