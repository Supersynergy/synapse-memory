use super::Plan;
use std::collections::HashMap;

/// Welford online mean + variance per plan.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OnlineStats {
    pub count: u64,
    pub mean: f64,
    pub m2: f64,
}

impl OnlineStats {
    pub fn new() -> Self {
        Self {
            count: 0,
            mean: 0.0,
            m2: 0.0,
        }
    }

    pub fn update(&mut self, x: f64) {
        self.count += 1;
        let delta = x - self.mean;
        self.mean += delta / self.count as f64;
        let delta2 = x - self.mean;
        self.m2 += delta * delta2;
    }

    pub fn variance(&self) -> f64 {
        if self.count < 2 {
            1e9
        } else {
            self.m2 / (self.count - 1) as f64
        }
    }
}

impl Default for OnlineStats {
    fn default() -> Self {
        Self::new()
    }
}

/// ε-greedy bandit with latency-weighted arm selection.
/// Lower mean latency = better. Explore 10%, exploit 90%.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ThompsonSampler {
    pub stats: HashMap<Plan, OnlineStats>,
    rng_state: u64,
}

impl ThompsonSampler {
    pub fn new() -> Self {
        Self {
            stats: HashMap::new(),
            rng_state: 0xdeadbeef_cafef00d,
        }
    }

    /// Fast xorshift64 — no rand dep needed.
    fn rand_f64(&mut self) -> f64 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng_state = x;
        (x >> 11) as f64 / (u64::MAX >> 11) as f64
    }

    /// Choose best plan from candidates. 10% explore (random), 90% exploit (lowest mean).
    pub fn choose(&mut self, candidates: &[Plan]) -> Plan {
        if candidates.is_empty() {
            return Plan::MmapScanFull;
        }
        if candidates.len() == 1 {
            return candidates[0];
        }

        // Explore
        if self.rand_f64() < 0.10 {
            let idx = (self.rand_f64() * candidates.len() as f64) as usize;
            return candidates[idx.min(candidates.len() - 1)];
        }

        // Exploit: pick plan with lowest mean latency
        candidates
            .iter()
            .min_by(|a, b| {
                let ma = self.stats.get(a).map(|s| s.mean).unwrap_or(f64::MAX);
                let mb = self.stats.get(b).map(|s| s.mean).unwrap_or(f64::MAX);
                ma.partial_cmp(&mb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied()
            .unwrap_or(Plan::MmapScanFull)
    }

    pub fn record(&mut self, plan: Plan, latency_us: u64) {
        self.stats
            .entry(plan)
            .or_default()
            .update(latency_us as f64);
    }

    /// Winrate: fraction of recorded executions where this plan was fastest vs all others.
    /// Approximated as: count(plan) observations where mean(plan) < mean of all others.
    pub fn winrate(&self, plan: Plan) -> f64 {
        let mine = match self.stats.get(&plan) {
            Some(s) if s.count > 0 => s.mean,
            _ => return 0.0,
        };
        let n_better = self
            .stats
            .iter()
            .filter(|(&p, s)| p != plan && s.mean < mine)
            .count();
        if n_better == 0 {
            1.0
        } else {
            0.0
        }
    }
}

impl Default for ThompsonSampler {
    fn default() -> Self {
        Self::new()
    }
}
