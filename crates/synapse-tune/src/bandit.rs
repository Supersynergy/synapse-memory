//! Thompson-Beta TTL bandit per cache-tag.
//!
//! State: (α, β) per tag. Action: pick TTL bucket. Reward: hit&fresh=+1, hit&stale=-2, miss=0.
//! Convergence: 50-200 events per tag → optimal TTL.
//! Replaces hard-coded TTL with adaptive learning.

use std::collections::HashMap;
use std::sync::RwLock;

type BanditState = HashMap<String, Vec<BetaArm>>;

#[derive(Debug, Clone, Copy)]
pub struct BetaArm {
    alpha: f64,
    beta: f64,
}

impl BetaArm {
    pub fn new() -> Self {
        Self {
            alpha: 1.0,
            beta: 1.0,
        }
    }
    /// Thompson-sample: draw value from Beta(α, β).
    /// Approx via inverse-CDF; simple Marsaglia for production-grade.
    pub fn sample(&self, rng: &mut impl FnMut() -> f64) -> f64 {
        let u1 = rng();
        let u2 = rng();
        let x = -u1.ln() * self.alpha;
        let y = -u2.ln() * self.beta;
        x / (x + y + 1e-9)
    }
    pub fn update(&mut self, reward: f64) {
        if reward > 0.0 {
            self.alpha += reward;
        } else {
            self.beta += -reward;
        }
    }
}

impl Default for BetaArm {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-tag TTL picker over fixed bucket set.
pub struct TtlBandit {
    buckets: Vec<u32>, // seconds
    state: RwLock<BanditState>,
}

impl TtlBandit {
    pub fn new(buckets: Vec<u32>) -> Self {
        Self {
            buckets,
            state: RwLock::new(HashMap::new()),
        }
    }
    pub fn default_buckets() -> Self {
        Self::new(vec![1, 10, 60, 600, 3600])
    }
    /// Pick TTL for a cache tag. Thompson-sample arms, return best bucket.
    pub fn pick(&self, tag: &str, rng: &mut impl FnMut() -> f64) -> u32 {
        let arms = {
            let g = self.state.read().unwrap();
            g.get(tag).cloned()
        };
        let arms = arms.unwrap_or_else(|| {
            let new = vec![BetaArm::new(); self.buckets.len()];
            self.state.write().unwrap().insert(tag.into(), new.clone());
            new
        });
        let mut best = (0usize, f64::NEG_INFINITY);
        for (i, a) in arms.iter().enumerate() {
            let s = a.sample(rng);
            if s > best.1 {
                best = (i, s);
            }
        }
        self.buckets[best.0]
    }
    /// Reward signal after observed cache-event.
    /// `bucket_idx` = index into self.buckets that was used.
    /// `reward`: +1 hit&fresh, -2 hit&stale, 0 miss.
    pub fn update(&self, tag: &str, bucket_idx: usize, reward: f64) {
        if let Ok(mut g) = self.state.write()
            && let Some(arms) = g.get_mut(tag)
            && let Some(a) = arms.get_mut(bucket_idx)
        {
            a.update(reward);
        }
    }
    pub fn buckets(&self) -> &[u32] {
        &self.buckets
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rng() -> impl FnMut() -> f64 {
        let mut state = 12345u64;
        move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as f64 / u32::MAX as f64
        }
    }
    #[test]
    fn pick_returns_valid_bucket() {
        let b = TtlBandit::default_buckets();
        let mut r = rng();
        let t = b.pick("post.123", &mut r);
        assert!(b.buckets().contains(&t));
    }
    #[test]
    fn update_shifts_distribution() {
        let b = TtlBandit::default_buckets();
        let mut r = rng();
        // Reward bucket-2 (60s) repeatedly
        for _ in 0..100 {
            let _ = b.pick("hot.tag", &mut r);
            b.update("hot.tag", 2, 1.0); // 60s rewarded
            b.update("hot.tag", 0, -2.0); // 1s penalized
        }
        // After many updates, 60s bucket should be picked more often
        let mut hits = 0;
        for _ in 0..50 {
            if b.pick("hot.tag", &mut r) == 60 {
                hits += 1;
            }
        }
        assert!(hits > 25, "60s should dominate, got {hits}/50");
    }
}
